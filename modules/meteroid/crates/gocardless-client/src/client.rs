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

/// Upper bound on how long we'll honor a `Retry-After`. This runs on a
/// pgmq/webhook worker, so an oversized (or hostile) header must not park the
/// task for minutes and hold its resources.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// A transport-level error worth another attempt: a timeout or a transient
/// client/connection failure. Non-transport errors (e.g. decode) are not.
fn is_transport_retryable(err: &GoCardlessError) -> bool {
    matches!(
        err,
        GoCardlessError::Timeout | GoCardlessError::ClientError(_)
    )
}

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
        let req = match self.build_request(Method::GET, path, access_token, None) {
            Ok(r) => r,
            Err(e) => return self.err(e),
        };
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
        let req = match self.build_request(Method::POST, path, access_token, idempotency_key) {
            Ok(r) => r,
            Err(e) => return self.err(e),
        };
        let req = req.header("Content-Type", "application/json").body(body);
        self.execute(req, retry)
    }

    /// POST that creates a resource, with idempotent-conflict recovery.
    ///
    /// On a 409 `idempotent_creation_conflict` — the create already succeeded on
    /// a prior attempt reusing this Idempotency-Key — GET `{resource_path}/{id}`
    /// of the conflicting resource and return it as success. `resource_path` is
    /// the collection path, e.g. `/payments`.
    pub(crate) fn post_create<T: DeserializeOwned + Send + 'static, B: Serialize>(
        &self,
        resource_path: &str,
        body: B,
        access_token: &SecretString,
        idempotency_key: &str,
        retry: RetryStrategy,
    ) -> Response<T> {
        let post = self.post::<T, B>(
            resource_path,
            body,
            access_token,
            Some(idempotency_key),
            retry.clone(),
        );
        let client = self.clone();
        let path = resource_path.to_string();
        let token = access_token.clone();
        Box::pin(async move {
            match post.await {
                Err(GoCardlessError::Api(err)) if err.is_idempotent_creation_conflict() => {
                    match err.conflicting_resource_id().map(str::to_owned) {
                        Some(id) => {
                            client
                                .get::<T>(&format!("{path}/{id}"), &token, retry)
                                .await
                        }
                        None => Err(GoCardlessError::Api(err)),
                    }
                }
                other => other,
            }
        })
    }

    /// DELETE — for un-registering webhook endpoints.
    #[allow(dead_code)]
    pub(crate) fn delete<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        access_token: &SecretString,
        retry: RetryStrategy,
    ) -> Response<T> {
        let req = match self.build_request(Method::DELETE, path, access_token, None) {
            Ok(r) => r,
            Err(e) => return self.err(e),
        };
        self.execute(req, retry)
    }

    fn build_request(
        &self,
        method: Method,
        path: &str,
        access_token: &SecretString,
        idempotency_key: Option<&str>,
    ) -> Result<RequestBuilder, GoCardlessError> {
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
        // A key that can't become a header value must fail loudly: silently
        // dropping it would send an unprotected POST and defeat idempotency.
        if let Some(key) = idempotency_key {
            let val = HeaderValue::from_str(key).map_err(|_| {
                crate::error::client_validation(format!(
                    "invalid idempotency key {key:?}: not a valid HTTP header value"
                ))
            })?;
            headers.insert("Idempotency-Key", val);
        }
        Ok(self
            .client
            .request(method, url)
            .headers(headers)
            .bearer_auth(access_token.expose_secret()))
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
                        let retryable = is_transport_retryable(&err);
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
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(Duration::from_secs)
                    // Clamp so a large/hostile header can't park the worker task.
                    .map(|d| d.min(MAX_RETRY_AFTER));
                let bytes = match response.bytes().await {
                    Ok(b) => b,
                    Err(e) => {
                        let err: GoCardlessError = e.into();
                        let retryable = is_transport_retryable(&err);
                        last_err = Some(err);
                        if !retryable || attempt + 1 == attempts {
                            break;
                        }
                        tokio::time::sleep(delay).await;
                        delay *= 2;
                        continue;
                    }
                };

                if status.is_success() {
                    let parsed: T = serde_json::from_slice(&bytes).map_err(|e| {
                        GoCardlessError::ClientError(format!(
                            "failed to decode gocardless response: {e}"
                        ))
                    })?;
                    return Ok(parsed);
                }

                // 429: rate-limited, retryable honoring Retry-After if present.
                // Other 4xx: surface as Api error, not retryable.
                // 5xx: retryable if attempts remain.
                let mut req_err = serde_json::from_slice::<ErrorResponse>(&bytes)
                    .map(|er| er.error)
                    .unwrap_or_default();
                req_err.http_status = status.as_u16();

                let is_rate_limited = status.as_u16() == 429;
                if (status.is_server_error() || is_rate_limited) && attempt + 1 < attempts {
                    last_err = Some(GoCardlessError::Api(req_err));
                    let wait = if is_rate_limited {
                        retry_after.unwrap_or(delay)
                    } else {
                        delay
                    };
                    tokio::time::sleep(wait).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn test_client() -> GoCardlessClient {
        GoCardlessClient::from_parts(
            "https://example.invalid",
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
    }

    /// A key that can't become an HTTP header value (e.g. contains a
    /// newline) must hard-error before any request is sent — silently
    /// dropping it would send an unprotected POST and defeat idempotency.
    #[test]
    fn invalid_idempotency_key_is_a_hard_error() {
        let err = test_client()
            .build_request(
                Method::POST,
                "/payments",
                &SecretString::from("token"),
                Some("bad\nkey"),
            )
            .expect_err("a header-invalid idempotency key must not be silently dropped");

        match err {
            GoCardlessError::Encode(_) => {}
            other => panic!("expected a client_validation (Encode) error, got {other:?}"),
        }
    }

    #[test]
    fn valid_idempotency_key_is_accepted() {
        let req = test_client().build_request(
            Method::POST,
            "/payments",
            &SecretString::from("token"),
            Some("stable-key-123"),
        );
        assert!(req.is_ok());
    }

    #[test]
    fn no_idempotency_key_is_accepted() {
        let req = test_client().build_request(
            Method::GET,
            "/payments/PM123",
            &SecretString::from("token"),
            None,
        );
        assert!(req.is_ok());
    }
}
