use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use tokio::net::TcpStream;
use url::Url;

use crate::error::EgressError;
use crate::policy::{DestinationPolicy, PolicyResolver};
use crate::url_rules::validate_url;

/// Enough for the error message a receiver sends back, and no more: whatever is read
/// here ends up visible to the tenant in its delivery log.
const MAX_BODY_BYTES: usize = 1024;

#[derive(Clone)]
pub struct EgressClientConfig {
    /// Where requests may go; without one, any destination is reachable.
    pub policy: Option<Arc<dyn DestinationPolicy>>,
    /// CONNECT egress proxy (Smokescreen-style) all traffic is sent through.
    pub proxy_url: Option<String>,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    pub user_agent: String,
}

impl Default for EgressClientConfig {
    fn default() -> Self {
        Self {
            policy: None,
            proxy_url: None,
            connect_timeout: Duration::from_secs(5),
            timeout: Duration::from_secs(15),
            user_agent: "Meteroid-Webhooks/1.0".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EgressResponse {
    pub status: u16,
    /// First `MAX_BODY_BYTES` of the response, lossily decoded as UTF-8.
    pub body_prefix: String,
    pub duration: Duration,
}

#[derive(Clone)]
pub struct EgressClient {
    client: reqwest::Client,
    policy: Option<Arc<dyn DestinationPolicy>>,
    proxy: Option<ProxyProbe>,
}

#[derive(Clone)]
struct ProxyProbe {
    host: String,
    port: u16,
    timeout: Duration,
}

impl ProxyProbe {
    fn new(url: &Url, timeout: Duration) -> Result<Self, EgressError> {
        let host = url
            .host_str()
            .ok_or_else(|| EgressError::ClientBuild("proxy url has no host".to_string()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| EgressError::ClientBuild("proxy url has no port".to_string()))?;
        Ok(Self {
            host: host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string(),
            port,
            timeout,
        })
    }

    async fn check(&self) -> Result<(), String> {
        match tokio::time::timeout(
            self.timeout,
            TcpStream::connect((self.host.as_str(), self.port)),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(err)) => Err(format!("could not connect: {err}")),
            Err(_) => Err("connection timed out".to_string()),
        }
    }
}

impl EgressClient {
    pub fn new(config: EgressClientConfig) -> Result<Self, EgressError> {
        let proxy = config
            .proxy_url
            .as_deref()
            .map(Url::parse)
            .transpose()
            .map_err(|err| EgressError::ClientBuild(format!("invalid proxy url: {err}")))?;

        let proxy_probe = proxy
            .as_ref()
            .map(|url| ProxyProbe::new(url, config.connect_timeout))
            .transpose()?;

        let user_agent = HeaderValue::from_str(&config.user_agent)
            .map_err(|err| EgressError::ClientBuild(err.to_string()))?;

        let mut builder = http_client_tracing::client_builder()
            .redirect(Policy::none())
            .connect_timeout(config.connect_timeout)
            .timeout(config.timeout)
            .user_agent(user_agent);

        if let Some(policy) = &config.policy {
            builder = builder.dns_resolver(Arc::new(PolicyResolver {
                policy: Arc::clone(policy),
                proxy_host: proxy
                    .as_ref()
                    .and_then(|url| url.host_str())
                    .map(str::to_string),
            }));
        }

        if let Some(proxy_url) = proxy {
            let proxy = reqwest::Proxy::all(proxy_url)
                .map_err(|err| EgressError::ClientBuild(err.to_string()))?;
            builder = builder.proxy(proxy);
        } else {
            builder = builder.no_proxy();
        }

        let client = builder
            .build()
            .map_err(|err| EgressError::ClientBuild(err.to_string()))?;

        Ok(Self {
            client,
            policy: config.policy,
            proxy: proxy_probe,
        })
    }

    /// The client for call sites that drive the request themselves: no redirects, and
    /// connections resolved through the policy.
    pub fn reqwest(&self) -> &reqwest::Client {
        &self.client
    }

    /// True without a policy: the delivery log then keeps no response body, which an internal
    /// service could otherwise be read back through.
    pub fn allows_private_destinations(&self) -> bool {
        self.policy
            .as_ref()
            .is_none_or(|policy| policy.allows_private_destinations())
    }

    pub fn validate_url(&self, url: &Url) -> Result<(), EgressError> {
        validate_url(url, self.allows_private_destinations())?;
        match &self.policy {
            Some(policy) => policy.check_destination(url),
            None => Ok(()),
        }
    }

    pub async fn post_json(
        &self,
        url: &Url,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<EgressResponse, EgressError> {
        self.validate_url(url)?;

        // With a CONNECT proxy the target host is resolved by the proxy, so the
        // policy's resolver never sees it — check it here instead.
        if self.proxy.is_some()
            && let Some(policy) = self
                .policy
                .as_ref()
                .filter(|policy| !policy.allows_private_destinations())
        {
            let host = url
                .host_str()
                .ok_or_else(|| EgressError::DisallowedHost("missing host".to_string()))?;
            policy.lookup(host.to_string(), true).await?;
        }

        let header_map = build_headers(headers)?;
        let started = Instant::now();

        let request = self.client.post(url.clone()).headers(header_map).body(body);
        let response = http_client_tracing::send(request).await;

        let response = match response {
            Ok(response) => response,
            Err(err) => {
                return Err(self
                    .blame_proxy_if_down(EgressError::from_reqwest(err))
                    .await);
            }
        };

        let status = response.status().as_u16();
        let body_prefix = read_body_prefix(response).await?;

        Ok(EgressResponse {
            status,
            body_prefix,
            duration: started.elapsed(),
        })
    }

    /// Behind a proxy every connection goes to it, so a failed request does not say whose
    /// fault it is. A fresh connection to the proxy tells the two apart.
    async fn blame_proxy_if_down(&self, err: EgressError) -> EgressError {
        let Some(proxy) = &self.proxy else {
            return err;
        };
        match proxy.check().await {
            Ok(()) => err,
            Err(reason) => EgressError::ProxyUnavailable(format!("{reason} (request: {err})")),
        }
    }
}

fn build_headers(headers: Vec<(String, String)>) -> Result<HeaderMap, EgressError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header_name = HeaderName::try_from(name.as_str())
            .map_err(|_| EgressError::InvalidHeader(name.clone()))?;
        let header_value =
            HeaderValue::from_str(&value).map_err(|_| EgressError::InvalidHeader(name))?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

async fn read_body_prefix(mut response: reqwest::Response) -> Result<String, EgressError> {
    let mut buffer: Vec<u8> = Vec::new();
    while buffer.len() < MAX_BODY_BYTES {
        match response.chunk().await.map_err(EgressError::from_reqwest)? {
            Some(chunk) => buffer.extend_from_slice(&chunk),
            None => break,
        }
    }
    buffer.truncate(MAX_BODY_BYTES);
    Ok(decode_body_prefix(&buffer))
}

/// NUL bytes are dropped because Postgres TEXT columns reject them.
fn decode_body_prefix(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\0', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_client() -> EgressClient {
        EgressClient::new(EgressClientConfig {
            connect_timeout: Duration::from_secs(2),
            timeout: Duration::from_secs(2),
            ..EgressClientConfig::default()
        })
        .expect("a local client builds")
    }

    async fn transport_error(url: String) -> String {
        match local_client()
            .post_json(&Url::parse(&url).unwrap(), vec![], vec![])
            .await
        {
            Err(EgressError::Transport(message)) => message,
            other => panic!("expected a transport error for {url}, got {other:?}"),
        }
    }

    /// The message reaches the tenant: it carries the cause, and neither the url nor the
    /// address it resolved to.
    fn assert_no_destination(message: &str, port: u16) {
        assert!(!message.contains("127.0.0.1"), "{message}");
        assert!(!message.contains(&port.to_string()), "{message}");
    }

    #[tokio::test]
    async fn a_refused_connection_says_so() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };

        let message = transport_error(format!("http://127.0.0.1:{port}/hook")).await;

        assert!(message.starts_with("could not connect: "), "{message}");
        assert!(message.to_lowercase().contains("refused"), "{message}");
        assert_no_destination(&message, port);
    }

    #[tokio::test]
    async fn a_dropped_connection_says_so() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                drop(stream);
            }
        });

        let message = transport_error(format!("http://127.0.0.1:{port}/hook")).await;

        assert_eq!(
            message,
            "request failed: connection closed before message completed"
        );
    }

    #[tokio::test]
    async fn a_failed_tls_handshake_says_so() {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\nnot tls").await;
            }
        });

        let message = transport_error(format!("https://127.0.0.1:{port}/hook")).await;

        assert!(message.starts_with("could not connect: "), "{message}");
        assert_ne!(message, "could not connect: ", "{message}");
        assert_no_destination(&message, port);
    }

    fn client_behind(proxy_port: u16) -> EgressClient {
        EgressClient::new(EgressClientConfig {
            proxy_url: Some(format!("http://127.0.0.1:{proxy_port}")),
            connect_timeout: Duration::from_secs(2),
            timeout: Duration::from_secs(2),
            ..EgressClientConfig::default()
        })
        .unwrap()
    }

    /// A proxy that answers every CONNECT with `response`.
    async fn proxy_answering(response: &'static [u8]) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request).await;
                let _ = stream.write_all(response).await;
            }
        });
        port
    }

    #[tokio::test]
    async fn a_proxy_that_refuses_connections_is_blamed() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };

        let result = client_behind(port)
            .post_json(
                &Url::parse("https://example.com/hook").unwrap(),
                vec![],
                vec![],
            )
            .await;

        assert!(
            matches!(result, Err(EgressError::ProxyUnavailable(_))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn a_tunnel_refused_by_a_live_proxy_is_the_destination_s_failure() {
        let port = proxy_answering(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n").await;

        let result = client_behind(port)
            .post_json(
                &Url::parse("https://example.com/hook").unwrap(),
                vec![],
                vec![],
            )
            .await;

        assert!(
            matches!(result, Err(EgressError::Transport(_))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn a_destination_refused_by_the_proxy_is_reported_as_such() {
        let port =
            proxy_answering(b"HTTP/1.1 407 Request Rejected by Proxy\r\ncontent-length: 0\r\n\r\n")
                .await;

        let result = client_behind(port)
            .post_json(
                &Url::parse("https://example.com/hook").unwrap(),
                vec![],
                vec![],
            )
            .await;

        assert!(
            matches!(result, Err(EgressError::ProxyRefused)),
            "{result:?}"
        );
    }

    #[test]
    fn body_prefix_drops_nul_bytes() {
        assert_eq!(decode_body_prefix(b"ok\0body\0"), "okbody");
        assert_eq!(decode_body_prefix(b"\xff\0x"), "\u{fffd}x");
    }
}
