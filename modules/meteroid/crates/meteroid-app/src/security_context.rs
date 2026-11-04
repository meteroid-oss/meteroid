//! Extracts the client request context (IP, user agent + light parsing)
//! for security-audit-log entries. Handlers call `from_request` directly
//! rather than going through a tonic layer — avoids the threading-through
//! a request extension and keeps anonymous (login) and authenticated
//! handlers symmetrical.

use meteroid_store::domain::org_security_audit_log::SecurityRequestContext;
use tonic::Request;

pub const XFF_HEADER: &str = "x-forwarded-for";
pub const REAL_IP_HEADER: &str = "x-real-ip";
const UA_HEADER: &str = "user-agent";

/// Resolves the client IP from an `X-Forwarded-For` value, trusting only the
/// proxies we control. Each proxy *appends* the peer it observed, so the
/// rightmost entry is what our innermost trusted proxy actually saw — the
/// leftmost is fully client-supplied and trivially spoofable. `trusted_hops`
/// skips additional trailing entries added by trusted infra chained in front
/// of that proxy. Returns `None` if the header has fewer entries than expected.
pub fn client_ip_from_xff(xff: &str, trusted_hops: usize) -> Option<String> {
    let entries: Vec<&str> = xff
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    entries
        .len()
        .checked_sub(1 + trusted_hops)
        .map(|idx| entries[idx].to_string())
}

fn resolve_client_ip(
    xff: Option<&str>,
    real_ip: Option<&str>,
    remote: Option<std::net::SocketAddr>,
    trusted_hops: impl FnOnce() -> usize,
) -> Option<String> {
    xff.and_then(|s| client_ip_from_xff(s, trusted_hops()))
        .or_else(|| {
            real_ip
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| remote.map(|a| a.ip().to_string()))
}

/// Same resolution as [`from_request`], for tower layers that see the raw HTTP request.
pub fn client_ip_from_http<B>(request: &http::Request<B>, trusted_hops: usize) -> Option<String> {
    let headers = request.headers();
    let remote = request
        .extensions()
        .get::<tonic::transport::server::TcpConnectInfo>()
        .and_then(|info| info.remote_addr());
    resolve_client_ip(
        headers.get(XFF_HEADER).and_then(|v| v.to_str().ok()),
        headers.get(REAL_IP_HEADER).and_then(|v| v.to_str().ok()),
        remote,
        || trusted_hops,
    )
}

/// `METEROID_TRUSTED_PROXY_HOPS`, set on every request by the API server.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrustedProxyHops(pub usize);

pub fn from_request<T>(request: &Request<T>) -> SecurityRequestContext {
    let metadata = request.metadata();

    let ip_address = resolve_client_ip(
        metadata.get(XFF_HEADER).and_then(|v| v.to_str().ok()),
        metadata.get(REAL_IP_HEADER).and_then(|v| v.to_str().ok()),
        request.remote_addr(),
        || {
            request
                .extensions()
                .get::<TrustedProxyHops>()
                .copied()
                .unwrap_or_default()
                .0
        },
    );

    let user_agent = metadata
        .get(UA_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    build(ip_address, user_agent)
}

/// Build a context from already-extracted IP and UA strings. Used by the
/// axum-side OAuth callback, which doesn't go through tonic.
pub fn build(ip_address: Option<String>, user_agent: Option<String>) -> SecurityRequestContext {
    let (ua_browser, ua_os, ua_device) = user_agent
        .as_deref()
        .map(parse_user_agent)
        .unwrap_or((None, None, None));

    SecurityRequestContext {
        ip_address,
        user_agent,
        ua_browser,
        ua_os,
        ua_device,
    }
}

/// Lightweight UA parsing — identifies the major browser, OS, and device
/// class. Not RFC-compliant, but stable enough to surface "Chrome on macOS,
/// desktop" in the audit UI without pulling in a full UA parsing crate
/// (which carries hundreds of regex rules updated continuously).
fn parse_user_agent(ua: &str) -> (Option<String>, Option<String>, Option<String>) {
    let lower = ua.to_lowercase();

    // Order matters: Edge advertises Chrome/Safari, Chrome advertises Safari.
    let browser = if lower.contains("edg/") || lower.contains("edge/") {
        Some("Edge")
    } else if lower.contains("opr/") || lower.contains("opera/") {
        Some("Opera")
    } else if lower.contains("firefox/") {
        Some("Firefox")
    } else if lower.contains("chrome/") || lower.contains("crios/") {
        Some("Chrome")
    } else if lower.contains("safari/") {
        Some("Safari")
    } else {
        None
    };

    // iOS / Android must beat the generic desktop checks.
    let os = if lower.contains("iphone") || lower.contains("ipad") || lower.contains("ipod") {
        Some("iOS")
    } else if lower.contains("android") {
        Some("Android")
    } else if lower.contains("windows") {
        Some("Windows")
    } else if lower.contains("mac os") || lower.contains("macos") {
        Some("macOS")
    } else if lower.contains("linux") {
        Some("Linux")
    } else {
        None
    };

    let device = if lower.contains("iphone")
        || lower.contains("android")
        || lower.contains("mobile")
        || lower.contains("ipod")
    {
        Some("Mobile")
    } else if lower.contains("ipad") || lower.contains("tablet") {
        Some("Tablet")
    } else {
        Some("Desktop")
    };

    (
        browser.map(String::from),
        os.map(String::from),
        device.map(String::from),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chrome_macos_desktop() {
        let (b, o, d) = parse_user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36",
        );
        assert_eq!(b.as_deref(), Some("Chrome"));
        assert_eq!(o.as_deref(), Some("macOS"));
        assert_eq!(d.as_deref(), Some("Desktop"));
    }

    #[test]
    fn parses_safari_iphone_mobile() {
        let (b, o, d) = parse_user_agent(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1",
        );
        assert_eq!(b.as_deref(), Some("Safari"));
        assert_eq!(o.as_deref(), Some("iOS"));
        assert_eq!(d.as_deref(), Some("Mobile"));
    }

    #[test]
    fn parses_firefox_windows() {
        let (b, o, d) = parse_user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:128.0) Gecko/20100101 Firefox/128.0",
        );
        assert_eq!(b.as_deref(), Some("Firefox"));
        assert_eq!(o.as_deref(), Some("Windows"));
        assert_eq!(d.as_deref(), Some("Desktop"));
    }

    #[test]
    fn xff_single_proxy_takes_rightmost() {
        // Traefik appends the real client; an attacker-prepended value must lose.
        assert_eq!(
            client_ip_from_xff("1.2.3.4, 203.0.113.7", 0).as_deref(),
            Some("203.0.113.7")
        );
        assert_eq!(
            client_ip_from_xff("203.0.113.7", 0).as_deref(),
            Some("203.0.113.7")
        );
    }

    #[test]
    fn xff_skips_trusted_hops() {
        // LB -> Traefik: chain is "client, lb"; one trusted hop -> client.
        assert_eq!(
            client_ip_from_xff("203.0.113.7, 10.0.0.1", 1).as_deref(),
            Some("203.0.113.7")
        );
    }

    #[test]
    fn xff_too_few_entries_falls_back() {
        assert_eq!(client_ip_from_xff("10.0.0.1", 1), None);
        assert_eq!(client_ip_from_xff("  ", 0), None);
    }

    #[test]
    fn parses_edge_over_chrome() {
        // Edge advertises "Chrome/138 Safari/537.36 Edg/138" — Edge should win.
        let (b, _, _) = parse_user_agent(
            "Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138 Safari/537.36 Edg/138",
        );
        assert_eq!(b.as_deref(), Some("Edge"));
    }
}
