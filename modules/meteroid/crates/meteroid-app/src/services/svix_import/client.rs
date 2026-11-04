//! The few read-only calls of the Svix REST API the import needs. The `svix` crate is no
//! longer a dependency.

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::ACCEPT;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::Url;

/// The largest page Svix serves.
const PAGE_LIMIT: &str = "250";
const MAX_ATTEMPTS: u32 = 4;
const MAX_ERROR_BODY_CHARS: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum SvixError {
    #[error("SVIX_SERVER_URL `{0}` is not an http(s) URL")]
    InvalidServerUrl(String),
    #[error("{context}: {source}")]
    Http {
        context: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("{context}: Svix answered {status}: {body}")]
    Status {
        context: String,
        status: StatusCode,
        body: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub struct SvixApplication {
    pub id: String,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvixEndpoint {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub description: Option<String>,
    /// `None` (or empty): every event type.
    #[serde(default)]
    pub filter_types: Option<Vec<String>>,
    #[serde(default)]
    pub channels: Option<Vec<String>>,
    #[serde(default)]
    pub disabled: Option<bool>,
    /// Replaced `rateLimit`; older Svix servers only have the latter.
    #[serde(default)]
    pub throttle_rate: Option<u32>,
    #[serde(default)]
    pub rate_limit: Option<u32>,
}

/// Svix never returns the value of a sensitive header, only its name.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SvixEndpointHeaders {
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub sensitive: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ListPage<T> {
    data: Vec<T>,
    #[serde(default)]
    iterator: Option<String>,
    #[serde(default)]
    done: bool,
}

#[derive(Deserialize)]
struct EndpointSecret {
    key: String,
}

pub struct SvixClient {
    http: reqwest::Client,
    base: Url,
    token: SecretString,
}

impl SvixClient {
    pub fn new(server_url: &str, token: SecretString) -> Result<Self, SvixError> {
        let server_url = server_url.trim();
        let base = Url::parse(server_url)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https") && !url.cannot_be_a_base())
            .ok_or_else(|| SvixError::InvalidServerUrl(server_url.to_string()))?;

        let http = http_client_tracing::client_builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|source| SvixError::Http {
                context: "cannot build the HTTP client".to_string(),
                source,
            })?;

        Ok(Self { http, base, token })
    }

    pub async fn list_applications(&self) -> Result<Vec<SvixApplication>, SvixError> {
        self.list_all(&["api", "v1", "app"], "cannot list the Svix applications")
            .await
    }

    pub async fn list_endpoints(&self, app_id: &str) -> Result<Vec<SvixEndpoint>, SvixError> {
        self.list_all(
            &["api", "v1", "app", app_id, "endpoint"],
            &format!("cannot list the endpoints of Svix application {app_id}"),
        )
        .await
    }

    pub async fn endpoint_headers(
        &self,
        app_id: &str,
        endpoint_id: &str,
    ) -> Result<SvixEndpointHeaders, SvixError> {
        let url = self.url(&[
            "api",
            "v1",
            "app",
            app_id,
            "endpoint",
            endpoint_id,
            "headers",
        ]);
        self.get(
            url,
            &format!("cannot read the headers of Svix endpoint {endpoint_id}"),
        )
        .await
    }

    pub async fn endpoint_secret(
        &self,
        app_id: &str,
        endpoint_id: &str,
    ) -> Result<SecretString, SvixError> {
        let url = self.url(&[
            "api",
            "v1",
            "app",
            app_id,
            "endpoint",
            endpoint_id,
            "secret",
        ]);
        let secret: EndpointSecret = self
            .get(
                url,
                &format!("cannot read the signing secret of Svix endpoint {endpoint_id}"),
            )
            .await?;
        Ok(SecretString::from(secret.key))
    }

    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        url.set_query(None);
        url.path_segments_mut()
            .expect("checked in SvixClient::new")
            .pop_if_empty()
            .extend(segments);
        url
    }

    async fn list_all<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        context: &str,
    ) -> Result<Vec<T>, SvixError> {
        let mut items = Vec::new();
        let mut iterator: Option<String> = None;

        loop {
            let mut url = self.url(segments);
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("limit", PAGE_LIMIT);
                if let Some(iterator) = &iterator {
                    query.append_pair("iterator", iterator);
                }
            }

            let page: ListPage<T> = self.get(url, context).await?;
            items.extend(page.data);

            // A non-advancing iterator would loop forever.
            if page.done || page.iterator.is_none() || page.iterator == iterator {
                break;
            }
            iterator = page.iterator;
        }

        Ok(items)
    }

    async fn get<T: DeserializeOwned>(&self, url: Url, context: &str) -> Result<T, SvixError> {
        let mut attempt = 0;

        loop {
            attempt += 1;
            let request = self
                .http
                .get(url.clone())
                .bearer_auth(self.token.expose_secret())
                .header(ACCEPT, "application/json");

            match http_client_tracing::send(request).await {
                Ok(response) if response.status().is_success() => {
                    return response
                        .json::<T>()
                        .await
                        .map_err(|source| SvixError::Http {
                            context: context.to_string(),
                            source,
                        });
                }
                Ok(response) if is_transient(response.status()) && attempt < MAX_ATTEMPTS => {}
                Ok(response) => {
                    let status = response.status();
                    let body: String = response
                        .text()
                        .await
                        .unwrap_or_default()
                        .chars()
                        .take(MAX_ERROR_BODY_CHARS)
                        .collect();
                    return Err(SvixError::Status {
                        context: context.to_string(),
                        status,
                        body,
                    });
                }
                Err(source)
                    if (source.is_connect() || source.is_timeout()) && attempt < MAX_ATTEMPTS => {}
                Err(source) => {
                    return Err(SvixError::Http {
                        context: context.to_string(),
                        source,
                    });
                }
            }

            tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
        }
    }
}

fn is_transient(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(server_url: &str) -> SvixClient {
        SvixClient::new(server_url, SecretString::from("token".to_string())).unwrap()
    }

    #[test]
    fn paths_are_appended_to_the_server_url() {
        assert_eq!(
            client("http://svix:8071")
                .url(&["api", "v1", "app"])
                .as_str(),
            "http://svix:8071/api/v1/app"
        );
        assert_eq!(
            client("https://example.com/svix/")
                .url(&["api", "v1", "app", "app_1", "endpoint"])
                .as_str(),
            "https://example.com/svix/api/v1/app/app_1/endpoint"
        );
    }

    #[test]
    fn only_http_server_urls_are_accepted() {
        let token = || SecretString::from("token".to_string());
        assert!(SvixClient::new("svix:8071", token()).is_err());
        assert!(SvixClient::new("mailto:ops@example.com", token()).is_err());
        assert!(SvixClient::new(" https://svix.example.com ", token()).is_ok());
    }

    #[test]
    fn endpoints_parse_from_svix_json() {
        let endpoint: SvixEndpoint = serde_json::from_str(
            r#"{"id":"ep_1","url":"https://a.example","description":"","filterTypes":null,
                "channels":null,"disabled":false,"rateLimit":10,"uid":null,"version":1,
                "createdAt":"2025-01-01T00:00:00Z","updatedAt":"2025-01-01T00:00:00Z",
                "metadata":{}}"#,
        )
        .unwrap();
        assert_eq!(endpoint.id, "ep_1");
        assert_eq!(endpoint.filter_types, None);
        assert_eq!(endpoint.rate_limit, Some(10));

        let headers: SvixEndpointHeaders = serde_json::from_str(
            r#"{"headers":{"X-Team":"billing"},"sensitive":["Authorization"]}"#,
        )
        .unwrap();
        assert_eq!(headers.headers.unwrap()["X-Team"], "billing");
        assert_eq!(
            headers.sensitive.unwrap(),
            vec!["Authorization".to_string()]
        );
    }
}
