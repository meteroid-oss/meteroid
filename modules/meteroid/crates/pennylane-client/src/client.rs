use crate::error::PennylaneError;
use bytes::Bytes;
use governor::{DefaultKeyedRateLimiter, Jitter, Quota, RateLimiter};
use nonzero_ext::nonzero;
use reqwest::{Client, Method, RequestBuilder, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

/// Company API v2, against the spec vendored in `openapi/pennylane-v2.json`.
///
/// Pennylane allows 25 requests per 5 seconds per token
/// (<https://pennylane.readme.io/docs/rate-limiting-1>): requests are paced per token.
#[derive(Debug, Clone)]
pub struct PennylaneClient {
    http: Client,
    rate_limiter: Arc<DefaultKeyedRateLimiter<String>>,
    api_base: Url,
    max_attempts: u32,
    backoff: Duration,
}

/// Pennylane does not deduplicate creations, so a create that may have gone through is never
/// sent twice: the caller looks it up by its external reference instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Retry {
    Transient,
    RateLimitedOnly,
}

const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

impl Default for PennylaneClient {
    fn default() -> Self {
        Self::new()
    }
}

impl PennylaneClient {
    pub fn new() -> Self {
        Self::from_parts(
            "https://app.pennylane.com",
            Duration::from_secs(5),
            Duration::from_secs(30),
            3,
            nonzero!(5u32),
        )
    }

    pub fn from_parts(
        url: &str,
        connect_timeout: Duration,
        timeout: Duration,
        max_attempts: u32,
        rps: NonZeroU32,
    ) -> Self {
        let http = http_client_tracing::client_builder()
            .connect_timeout(connect_timeout)
            .timeout(timeout)
            .build()
            .expect("invalid client config");

        Self {
            http,
            rate_limiter: Arc::new(RateLimiter::keyed(Quota::per_second(rps))),
            api_base: Url::parse(url).expect("invalid url"),
            max_attempts: max_attempts.max(1),
            backoff: Duration::from_millis(500),
        }
    }

    pub(crate) async fn get<Resp: DeserializeOwned, Q: Serialize>(
        &self,
        path: &str,
        access_token: &SecretString,
        query: &Q,
    ) -> Result<Resp, PennylaneError> {
        let url = self.url(path);
        let bytes = self
            .execute(access_token, Retry::Transient, || {
                self.http.get(url.clone()).query(query)
            })
            .await?;
        decode(&bytes)
    }

    pub(crate) async fn send_json<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        access_token: &SecretString,
        body: &Req,
    ) -> Result<Resp, PennylaneError> {
        let retry = if method == Method::POST {
            Retry::RateLimitedOnly
        } else {
            Retry::Transient
        };
        let url = self.url(path);
        let bytes = self
            .execute(access_token, retry, || {
                self.http.request(method.clone(), url.clone()).json(body)
            })
            .await?;
        decode(&bytes)
    }

    /// An idempotent call that returns no document.
    pub(crate) async fn put_empty(
        &self,
        path: &str,
        access_token: &SecretString,
    ) -> Result<(), PennylaneError> {
        let url = self.url(path);
        self.execute(access_token, Retry::Transient, || {
            self.http.put(url.clone())
        })
        .await
        .map(|_| ())
    }

    pub(crate) async fn post_multipart<Resp: DeserializeOwned>(
        &self,
        path: &str,
        access_token: &SecretString,
        retry: Retry,
        form: impl Fn() -> reqwest::multipart::Form,
    ) -> Result<Resp, PennylaneError> {
        let url = self.url(path);
        let bytes = self
            .execute(access_token, retry, || {
                self.http.post(url.clone()).multipart(form())
            })
            .await?;
        decode(&bytes)
    }

    fn url(&self, path: &str) -> Url {
        self.api_base.join(path).expect("invalid path")
    }

    async fn execute(
        &self,
        access_token: &SecretString,
        retry: Retry,
        build: impl Fn() -> RequestBuilder,
    ) -> Result<Bytes, PennylaneError> {
        let mut delay = self.backoff;
        let mut attempt = 1;
        loop {
            self.rate_limiter
                .until_key_ready_with_jitter(
                    &access_token.expose_secret().to_owned(),
                    Jitter::up_to(Duration::from_millis(200)),
                )
                .await;

            let request = build().bearer_auth(access_token.expose_secret());
            let err: PennylaneError = match http_client_tracing::send(request).await {
                Ok(response) => {
                    let status = response.status();
                    let retry_after = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                        .map(|secs| Duration::from_secs(secs).min(MAX_RETRY_AFTER));
                    match response.bytes().await {
                        Ok(body) if status.is_success() => return Ok(body),
                        Ok(body) => {
                            PennylaneError::from_response(status.as_u16(), &body, retry_after)
                        }
                        Err(err) => err.into(),
                    }
                }
                Err(err) => err.into(),
            };

            let rate_limited = err.status() == Some(429);
            let retryable = match retry {
                Retry::Transient => err.is_retryable(),
                Retry::RateLimitedOnly => rate_limited,
            };
            if !retryable || attempt >= self.max_attempts {
                return Err(err);
            }

            let wait = match &err {
                PennylaneError::Api {
                    retry_after: Some(retry_after),
                    ..
                } => *retry_after,
                _ => delay,
            };
            tokio::time::sleep(wait).await;
            delay *= 2;
            attempt += 1;
        }
    }
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, PennylaneError> {
    serde_json::from_slice(bytes).map_err(|err| PennylaneError::Decode(err.to_string()))
}
