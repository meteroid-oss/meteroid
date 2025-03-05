use crate::error::{ErrorResponse, StripeError};
use crate::request::{Outcome, RetryStrategy};
use bytes::Bytes;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, Method, RequestBuilder, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::{de::DeserializeOwned, Serialize};
use std::future;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub type Response<T> = Pin<Box<dyn Future<Output = Result<T, StripeError>> + Send>>;

static USER_AGENT: &str = concat!(
    "Meteroid/Stripe/v1 RustBindings/",
    env!("CARGO_PKG_VERSION")
);

static API_VERSION: &str = "2022-11-15";

#[derive(Debug, Clone)]
pub struct StripeClient {
    client: Client,
    headers: StripeHeaders,
    api_base: Url,
    api_root: String,
}

#[derive(Debug, Clone)]
pub struct StripeHeaders {
    pub stripe_version: String,
    pub user_agent: String,

    pub client_id: Option<String>,
    pub stripe_account: Option<String>,
}

impl StripeHeaders {
    pub fn as_header_map(&self) -> HeaderMap {
        let mut header_map = HeaderMap::with_capacity(4);

        let mut add_header = |name: &'static str, value: &'_ str| {
            header_map.insert(
                name,
                HeaderValue::from_str(value)
                    .unwrap_or_else(|_| panic!("Invalid {} header value", name)),
            )
        };

        if let Some(client_id) = &self.client_id {
            add_header("Client-Id", client_id);
        }
        if let Some(stripe_account) = &self.stripe_account {
            add_header("Stripe-Account", stripe_account);
        }
        add_header("Stripe-Version", &self.stripe_version);
        add_header("User-Agent", &self.user_agent);

        header_map
    }
}

impl StripeClient {
    pub fn new() -> Self {
        Self::from_parts(
            "https://api.stripe.com/",
            Duration::from_secs(5),
            Duration::from_secs(10),
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
            headers: StripeHeaders {
                stripe_version: API_VERSION.to_string(),
                user_agent: USER_AGENT.to_string(),
                client_id: None,
                stripe_account: None,
            },
            api_base: Url::parse(url.into()).expect("invalid url"),
            api_root: "v1".to_string(),
        }
    }

    pub(crate) fn get<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        secret_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);

        let request_builder = self.create_init_request(Method::GET, url, secret_key, None);

        self.execute(request_builder, retry_strategy)
    }

    pub(crate) fn post<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        secret_key: &SecretString,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);

        let request_builder = self.create_init_request(Method::POST, url, secret_key, None);

        self.execute(request_builder, retry_strategy)
    }

    /// Make a `POST` http request with urlencoded body
    pub(crate) fn post_form<T: DeserializeOwned + Send + 'static, F: Serialize>(
        &self,
        path: &str,
        form: F,
        secret_key: &'_ SecretString,
        idempotency_key: String,
        retry_strategy: RetryStrategy,
    ) -> Response<T> {
        let url = self.url(path);

        let mut params_buffer = Vec::new();
        let qs_ser = &mut serde_qs::Serializer::new(&mut params_buffer);

        if let Err(qs_ser_err) = serde_path_to_error::serialize(&form, qs_ser) {
            return self.err(StripeError::QueryStringSerialize(qs_ser_err));
        }

        let body = std::str::from_utf8(params_buffer.as_slice())
            .expect("Unable to extract string from params_buffer")
            .to_string();

        let request_builder = self
            .create_init_request(Method::POST, url, secret_key, Some(idempotency_key))
            .body(body);

        self.execute(request_builder, retry_strategy)
    }

    fn create_init_request(
        &self,
        method: Method,
        url: Url,
        secret_key: &SecretString,
        idempotency_key: Option<String>,
    ) -> RequestBuilder {
        let mut builder = self
            .client
            .request(method, url)
            .headers(self.headers.as_header_map())
            .bearer_auth(secret_key.expose_secret());

        if let Some(key) = idempotency_key {
            builder = builder.header("Idempotency-Key", key);
        }

        builder
    }

    #[allow(dead_code)]
    #[inline(always)]
    fn err<T: Send + 'static>(&self, err: StripeError) -> Response<T> {
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
            serde_path_to_error::deserialize(json_deserializer).map_err(StripeError::from)
        })
    }

    async fn send_inner(
        req_builder: RequestBuilder,
        retry_strategy: RetryStrategy,
    ) -> Result<Bytes, StripeError> {
        let mut tries: u32 = 0;

        loop {
            let response = req_builder
                .try_clone()
                .ok_or(StripeError::ClientError(
                    "streaming request is not supported".to_string(),
                ))?
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let resp_status = resp.status();
                    let resp_headers = resp.headers().clone();

                    let resp_bytes = resp.bytes().await?;

                    if resp_status.is_success() {
                        return Ok(resp_bytes);
                    } else {
                        let stripe_retry = resp_headers
                            .get("Stripe-Should-Retry")
                            .and_then(|s| s.to_str().ok())
                            .and_then(|s| s.parse().ok());

                        match retry_strategy.test(Some(resp_status), stripe_retry, tries) {
                            Outcome::Stop => {
                                let json_deserializer =
                                    &mut serde_json::Deserializer::from_slice(&resp_bytes);
                                let error = serde_path_to_error::deserialize(json_deserializer)
                                    .map(|mut e: ErrorResponse| {
                                        e.error.http_status = resp_status.into();
                                        StripeError::from(e.error)
                                    })
                                    .unwrap_or_else(StripeError::from);

                                return Err(error);
                            }
                            Outcome::Continue(sleep_duration) => {
                                tries += 1;
                                tokio::time::sleep(sleep_duration).await;
                                continue;
                            }
                        }
                    }
                }
                Err(err) => match retry_strategy.test(None, None, tries) {
                    Outcome::Stop => return Err(StripeError::from(err)),
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

impl Default for StripeClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::client::{StripeHeaders, API_VERSION, USER_AGENT};
    use reqwest::header::HeaderValue;

    #[test]
    fn test_stripe_headers() {
        let strategy = StripeHeaders {
            stripe_version: API_VERSION.to_string(),
            user_agent: USER_AGENT.to_string(),
            client_id: None,
            stripe_account: None,
        };

        let header_map = strategy.as_header_map();

        assert_eq!(
            header_map.get("Stripe-Version"),
            Some(&HeaderValue::from_static(API_VERSION))
        );
        assert_eq!(
            header_map.get("User-Agent"),
            Some(&HeaderValue::from_static(USER_AGENT))
        );
        assert_eq!(header_map.get("Client-Id"), None);
        assert_eq!(header_map.get("Stripe-Account"), None);
    }
}
