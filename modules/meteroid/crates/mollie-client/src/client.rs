use crate::error::{MollieError, RequestError};
use crate::request::{Outcome, RetryStrategy};
use bytes::Bytes;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, Method, RequestBuilder, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::{Serialize, de::DeserializeOwned};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub type Response<T> = Pin<Box<dyn Future<Output = Result<T, MollieError>> + Send>>;

static USER_AGENT: &str = concat!(
    "Meteroid/Mollie/v2 RustBindings/",
    env!("CARGO_PKG_VERSION")
);

/// The API key prefix selects test/live, so one client serves every connector.
#[derive(Debug, Clone)]
pub struct MollieClient {
    client: Client,
    api_base: Url,
    api_root: String,
}

impl MollieClient {
    /// Worst case (3 attempts × 12s + backoff) stays under the store's 45s provider timeout, so an
    /// accepted charge is never cut off mid-retry.
    pub fn new() -> Self {
        Self::from_parts(
            "https://api.mollie.com/",
            Duration::from_secs(5),
            Duration::from_secs(12),
        )
    }

    pub fn from_parts<'a>(
        url: impl Into<&'a str>,
        connect_timeout: Duration,
        timeout: Duration,
    ) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(connect_timeout)
                .timeout(timeout)
                .build()
                .expect("invalid client config"),
            api_base: Url::parse(url.into()).expect("invalid url"),
            api_root: "v2".to_string(),
        }
    }

    pub(crate) fn get<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        api_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);
        let request_builder = self.create_init_request(Method::GET, url, api_key);
        self.execute(request_builder, retry_strategy)
    }

    /// Every Mollie `POST` accepts an `Idempotency-Key` (kept for one hour).
    pub(crate) fn post_json<T: DeserializeOwned + Send + 'static, B: Serialize>(
        &self,
        path: &str,
        body: B,
        api_key: &SecretString,
        idempotency_key: Option<&str>,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);
        let mut request_builder = self
            .create_init_request(Method::POST, url, api_key)
            .json(&body);
        if let Some(key) = idempotency_key {
            request_builder = request_builder.header("Idempotency-Key", key);
        }
        self.execute(request_builder, retry_strategy)
    }

    pub(crate) fn patch_json<T: DeserializeOwned + Send + 'static, B: Serialize>(
        &self,
        path: &str,
        body: B,
        api_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);
        let request_builder = self
            .create_init_request(Method::PATCH, url, api_key)
            .json(&body);
        self.execute(request_builder, retry_strategy)
    }

    pub(crate) fn delete<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        api_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);
        let request_builder = self.create_init_request(Method::DELETE, url, api_key);
        self.execute(request_builder, retry_strategy)
    }

    fn create_init_request(
        &self,
        method: Method,
        url: Url,
        api_key: &SecretString,
    ) -> RequestBuilder {
        self.client
            .request(method, url)
            .headers(self.default_headers())
            .bearer_auth(api_key.expose_secret())
    }

    fn default_headers(&self) -> HeaderMap {
        let mut header_map = HeaderMap::with_capacity(2);
        header_map.insert("User-Agent", HeaderValue::from_static(USER_AGENT));
        header_map.insert("Accept", HeaderValue::from_static("application/json"));
        header_map
    }

    fn url(&self, path: &str) -> Url {
        let mut url = self.api_base.clone();
        url.set_path(&format!(
            "{}/{}",
            self.api_root,
            path.trim_start_matches('/')
        ));
        url
    }

    pub fn execute<T: DeserializeOwned + Send>(
        &self,
        request_builder: RequestBuilder,
        strategy: RetryStrategy,
    ) -> Response<T> {
        Box::pin(async move {
            let bytes = Self::send_inner(request_builder, strategy.clone()).await?;
            let json_deserializer = &mut serde_json::Deserializer::from_slice(&bytes);
            serde_path_to_error::deserialize(json_deserializer).map_err(MollieError::from)
        })
    }

    async fn send_inner(
        req_builder: RequestBuilder,
        retry_strategy: RetryStrategy,
    ) -> Result<Bytes, MollieError> {
        let mut tries: u32 = 0;

        loop {
            let response = req_builder
                .try_clone()
                .ok_or(MollieError::ClientError(
                    "streaming request is not supported".to_string(),
                ))?
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let resp_status = resp.status();
                    let resp_bytes = resp.bytes().await?;

                    if resp_status.is_success() {
                        return Ok(resp_bytes);
                    }
                    match retry_strategy.test(Some(resp_status), tries) {
                        Outcome::Stop => {
                            let json_deserializer =
                                &mut serde_json::Deserializer::from_slice(&resp_bytes);
                            let mut error: RequestError =
                                serde_path_to_error::deserialize(json_deserializer)
                                    .unwrap_or_default();
                            error.status = resp_status.into();
                            return Err(MollieError::from(error));
                        }
                        Outcome::Continue(sleep_duration) => {
                            tries += 1;
                            tokio::time::sleep(sleep_duration).await;
                            continue;
                        }
                    }
                }
                Err(err) => match retry_strategy.test(None, tries) {
                    Outcome::Stop => return Err(MollieError::from(err)),
                    Outcome::Continue(sleep_duration) => {
                        tries += 1;
                        tokio::time::sleep(sleep_duration).await;
                        continue;
                    }
                },
            }
        }
    }
}

impl Default for MollieClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::MollieClient;

    #[test]
    fn test_url_building() {
        let client = MollieClient::new();
        let url = client.url("/payments/tr_x");
        assert_eq!(url.as_str(), "https://api.mollie.com/v2/payments/tr_x");
    }
}
