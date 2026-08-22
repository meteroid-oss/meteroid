use crate::error::{RequestError, StancerError};
use crate::request::{Outcome, RetryStrategy};
use bytes::Bytes;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, Method, RequestBuilder, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::{Serialize, de::DeserializeOwned};
use std::future;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub type Response<T> = Pin<Box<dyn Future<Output = Result<T, StancerError>> + Send>>;

static USER_AGENT: &str = concat!(
    "Meteroid/Stancer/v1 RustBindings/",
    env!("CARGO_PKG_VERSION")
);

#[derive(Debug, Clone)]
pub struct StancerClient {
    client: Client,
    api_base: Url,
    api_root: String,
}

impl StancerClient {
    pub fn new() -> Self {
        Self::from_parts(
            "https://api.stancer.com/",
            Duration::from_secs(10),
            Duration::from_secs(30),
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
        secret_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);
        let request_builder = self.create_init_request(Method::GET, url, secret_key);
        self.execute(request_builder, retry_strategy)
    }

    /// Make a `GET` request with query parameters (list/search endpoints).
    pub(crate) fn get_with_query<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        secret_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let mut url = self.url(path);
        url.query_pairs_mut().extend_pairs(query);
        let request_builder = self.create_init_request(Method::GET, url, secret_key);
        self.execute(request_builder, retry_strategy)
    }

    /// Make a `POST` request with a JSON body — Stancer's API is plain JSON,
    /// unlike Stripe's form-encoded bodies.
    pub(crate) fn post_json<T: DeserializeOwned + Send + 'static, B: Serialize>(
        &self,
        path: &str,
        body: B,
        secret_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);

        let request_builder = self
            .create_init_request(Method::POST, url, secret_key)
            .json(&body);

        self.execute(request_builder, retry_strategy)
    }

    /// Make a `DELETE` request (Stancer returns the deleted/canceled resource
    /// as a JSON body, e.g. `DELETE /v2/payment_intents/{id}`).
    pub(crate) fn delete<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        secret_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);
        let request_builder = self.create_init_request(Method::DELETE, url, secret_key);
        self.execute(request_builder, retry_strategy)
    }

    /// Make a `PATCH` request with a JSON body (partial resource update).
    pub(crate) fn patch_json<T: DeserializeOwned + Send + 'static, B: Serialize>(
        &self,
        path: &str,
        body: B,
        secret_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);

        let request_builder = self
            .create_init_request(Method::PATCH, url, secret_key)
            .json(&body);

        self.execute(request_builder, retry_strategy)
    }

    fn create_init_request(
        &self,
        method: Method,
        url: Url,
        secret_key: &SecretString,
    ) -> RequestBuilder {
        self.client
            .request(method, url)
            .headers(self.default_headers())
            // Stancer authenticates via HTTP Basic Auth: the secret key as
            // username, no password.
            .basic_auth(secret_key.expose_secret(), None::<&str>)
    }

    fn default_headers(&self) -> HeaderMap {
        let mut header_map = HeaderMap::with_capacity(1);
        header_map.insert("User-Agent", HeaderValue::from_static(USER_AGENT));
        header_map
    }

    #[allow(dead_code)]
    #[inline(always)]
    fn err<T: Send + 'static>(&self, err: StancerError) -> Response<T> {
        Box::pin(future::ready(Err(err)))
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
            serde_path_to_error::deserialize(json_deserializer).map_err(StancerError::from)
        })
    }

    async fn send_inner(
        req_builder: RequestBuilder,
        retry_strategy: RetryStrategy,
    ) -> Result<Bytes, StancerError> {
        let mut tries: u32 = 0;

        loop {
            let response = req_builder
                .try_clone()
                .ok_or(StancerError::ClientError(
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
                    } else {
                        match retry_strategy.test(Some(resp_status), tries) {
                            Outcome::Stop => {
                                let json_deserializer =
                                    &mut serde_json::Deserializer::from_slice(&resp_bytes);
                                let error = serde_path_to_error::deserialize(json_deserializer)
                                    .map(|mut e: RequestError| {
                                        e.http_status = resp_status.into();
                                        e
                                    })
                                    .unwrap_or_else(|_| RequestError {
                                        http_status: resp_status.into(),
                                        detail: vec![],
                                    });

                                return Err(StancerError::from(error));
                            }
                            Outcome::Continue(sleep_duration) => {
                                tries += 1;
                                tokio::time::sleep(sleep_duration).await;
                                continue;
                            }
                        }
                    }
                }
                Err(err) => match retry_strategy.test(None, tries) {
                    Outcome::Stop => return Err(StancerError::from(err)),
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

impl Default for StancerClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::StancerClient;

    #[test]
    fn test_url_building() {
        let client = StancerClient::new();
        let url = client.url("/customers/");
        assert_eq!(url.as_str(), "https://api.stancer.com/v2/customers/");
    }
}
