use std::net::IpAddr;

/// How hyper-util reports a CONNECT answered with `407`; its error type is private.
const PROXY_REFUSED: &str = "tunnel error: proxy authorization required";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EgressError {
    #[error("scheme '{0}' is not allowed")]
    DisallowedScheme(String),

    #[error("port {0} is not allowed")]
    DisallowedPort(u16),

    #[error("host is not allowed: {0}")]
    DisallowedHost(String),

    #[error("url must not contain credentials")]
    CredentialsInUrl,

    #[error("destination address {0} is private or reserved")]
    PrivateAddress(IpAddr),

    #[error("could not resolve '{host}': {message}")]
    DnsFailure { host: String, message: String },

    #[error("invalid header: {0}")]
    InvalidHeader(String),

    #[error("request timed out")]
    Timeout,

    /// The egress proxy refused the destination (Smokescreen answers `407` when the
    /// address it resolved is private, or the host is on its deny list).
    #[error("egress proxy refused the destination")]
    ProxyRefused,

    /// The request failed and the egress proxy does not accept connections either: the
    /// failure is ours, not the destination's.
    #[error("egress proxy is unavailable: {0}")]
    ProxyUnavailable(String),

    /// What reqwest reports, with its root cause: see [`EgressError::from_reqwest`].
    #[error("{0}")]
    Transport(String),

    #[error("could not build the http client: {0}")]
    ClientBuild(String),
}

impl EgressError {
    pub(crate) fn from_reqwest(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            return Self::Timeout;
        }
        let mut source: Option<&(dyn std::error::Error + 'static)> =
            std::error::Error::source(&err);
        while let Some(current) = source {
            if let Some(egress) = current.downcast_ref::<EgressError>() {
                return egress.clone();
            }
            if current.to_string() == PROXY_REFUSED {
                return Self::ProxyRefused;
            }
            source = current.source();
        }
        Self::Transport(describe_transport(&err))
    }
}

/// reqwest's own message names the url and nothing else; the cause ("Connection refused",
/// a TLS alert, a dropped connection) is at the bottom of its source chain. Only the
/// `Display` of each error is used: hyper's `ConnectError` puts the resolved address in
/// its `Debug`, and the result is shown to the tenant.
fn describe_transport(err: &reqwest::Error) -> String {
    let stage = if err.is_connect() {
        "could not connect"
    } else if err.is_body() || err.is_decode() {
        "could not read the response"
    } else {
        "request failed"
    };

    let mut root: &(dyn std::error::Error + 'static) = err;
    while let Some(next) = root.source() {
        root = next;
    }

    if std::ptr::addr_eq(root, err as &(dyn std::error::Error + 'static)) {
        stage.to_string()
    } else {
        format!("{stage}: {root}")
    }
}
