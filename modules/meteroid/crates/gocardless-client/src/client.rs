use crate::error::{ErrorResponse, GoCardlessError};
use crate::request::RetryStrategy;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, Method, RequestBuilder, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub type Response<T> = Pin<Box<dyn Future<Output = Result<T, GoCardlessError>> + Send>>;

static USER_AGENT: &str = concat!(
    "Meteroid/GoCardless/v1 RustBindings/",
    env!("CARGO_PKG_VERSION")
);

/// Pinned API version. Bump only after testing — GoCardless versions can
/// change field shapes (e.g. `status` enum values), so this needs deliberate
/// rev'ing rather than tracking head.
static API_VERSION: &str = "2015-07-06";

#[derive(Debug, Clone)]
pub struct GoCardlessClient {
    client: Client,
    api_base: Url,
}

impl GoCardlessClient {
    /// Live API. Use `from_sandbox()` in tests.
    pub fn new() -> Self {
        Self::from_parts(
            "https://api.gocardless.com",
            Duration::from_secs(10),
            Duration::from_secs(30),
        )
    }

    pub fn from_sandbox() -> Self {
        Self::from_parts(
            "https://api-sandbox.gocardless.com",
            Duration::from_secs(10),
            Duration::from_secs(30),
        )
    }

    pub fn from_parts(base: &str, connect_timeout: Duration, timeout: Duration) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(connect_timeout)
                .timeout(timeout)
                .build()
                .expect("invalid gocardless client config"),
            api_base: Url::parse(base).expect("invalid url"),
        }
    }

    /// GET — no body, no idempotency.
    pub(crate) fn get<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        access_token: &SecretString,
        retry: RetryStrategy,
    ) -> Response<T> {
        let req = self.build_request(Method::GET, path, access_token, None);
        self.execute(req, retry)
    }

    /// POST with JSON body. GoCardless wraps every entity in a top-level
    /// key matching the resource (e.g. `{ "customers": { ... } }`); callers
    /// are responsible for shaping the body that way.
    pub(crate) fn post<T: DeserializeOwned + Send + 'static, B: Serialize>(
        &self,
        path: &str,
        body: B,
        access_token: &SecretString,
        idempotency_key: Option<&str>,
        retry: RetryStrategy,
    ) -> Response<T> {
        let body = match serde_json::to_vec(&body) {
            Ok(b) => b,
            Err(e) => return self.err(GoCardlessError::Encode(e)),
        };
        let req = self
            .build_request(Method::POST, path, access_token, idempotency_key)
            .header("Content-Type", "application/json")
            .body(body);
        self.execute(req, retry)
    }

    /// DELETE — for un-registering webhook endpoints.
    #[allow(dead_code)]
    pub(crate) fn delete<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        access_token: &SecretString,
        retry: RetryStrategy,
    ) -> Response<T> {
        let req = self.build_request(Method::DELETE, path, access_token, None);
        self.execute(req, retry)
    }

    fn build_request(
        &self,
        method: Method,
        path: &str,
        access_token: &SecretString,
        idempotency_key: Option<&str>,
    ) -> RequestBuilder {
        let url = {
            let mut u = self.api_base.clone();
            u.set_path(path);
            u
        };
        let mut headers = HeaderMap::new();
        headers.insert("GoCardless-Version", HeaderValue::from_static(API_VERSION));
        headers.insert("Accept", HeaderValue::from_static("application/json"));
        headers.insert(
            "User-Agent",
            HeaderValue::from_str(USER_AGENT).expect("valid user agent"),
        );
        if let Some(key) = idempotency_key {
            if let Ok(val) = HeaderValue::from_str(key) {
                headers.insert("Idempotency-Key", val);
            }
        }
        self.client
            .request(method, url)
            .headers(headers)
            .bearer_auth(access_token.expose_secret())
    }

    fn err<T: Send + 'static>(&self, e: GoCardlessError) -> Response<T> {
        Box::pin(std::future::ready(Err(e)))
    }

    fn execute<T: DeserializeOwned + Send + 'static>(
        &self,
        req: RequestBuilder,
        retry: RetryStrategy,
    ) -> Response<T> {
        Box::pin(async move {
            let attempts = match &retry {
                RetryStrategy::NoRetry => 1u8,
                RetryStrategy::Retry(p) => p.count.max(1),
            };
            let backoff = match &retry {
                RetryStrategy::NoRetry => Duration::from_millis(0),
                RetryStrategy::Retry(p) => match p.backoff {
                    crate::request::Backoff::Exponential(d) => d,
                },
            };

            let mut last_err: Option<GoCardlessError> = None;
            let mut delay = backoff;
            for attempt in 0..attempts {
                let try_req = req
                    .try_clone()
                    .expect("non-streaming requests are cloneable");

                let result = try_req.send().await;
                let response = match result {
                    Ok(r) => r,
                    Err(e) => {
                        let err: GoCardlessError = e.into();
                        let retryable = matches!(
                            err,
                            GoCardlessError::Timeout | GoCardlessError::ClientError(_)
                        );
                        last_err = Some(err);
                        if !retryable || attempt + 1 == attempts {
                            break;
                        }
                        tokio::time::sleep(delay).await;
                        delay *= 2;
                        continue;
                    }
                };

                let status = response.status();
                let bytes = response.bytes().await.map_err(GoCardlessError::from)?;

                if status.is_success() {
                    let parsed: T = serde_json::from_slice(&bytes).map_err(|e| {
                        GoCardlessError::ClientError(format!(
                            "failed to decode gocardless response: {e}"
                        ))
                    })?;
                    return Ok(parsed);
                }

                // 4xx: surface as Api error, not retryable.
                // 5xx: retryable if attempts remain.
                let mut req_err = serde_json::from_slice::<ErrorResponse>(&bytes)
                    .map(|er| er.error)
                    .unwrap_or_default();
                req_err.http_status = status.as_u16();

                if status.is_server_error() && attempt + 1 < attempts {
                    last_err = Some(GoCardlessError::Api(req_err));
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                    continue;
                }

                return Err(GoCardlessError::Api(req_err));
            }

            Err(last_err.unwrap_or(GoCardlessError::ClientError(
                "exhausted retries with no response".to_string(),
            )))
        })
    }
}

impl Default for GoCardlessClient {
    fn default() -> Self {
        Self::new()
    }
}
