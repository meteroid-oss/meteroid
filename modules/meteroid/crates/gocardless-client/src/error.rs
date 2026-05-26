//! Error taxonomy matching `stripe-client::error` — same `Timeout` /
//! `ClientError` / `Api` partitioning so the connector adapter can route
//! `Timeout`/`ClientError` to `Transport` (retryable) and `Api` to a logical
//! failure.

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GoCardlessError {
    /// Network-level failure (DNS, TCP, TLS, mid-flight disconnect). The
    /// caller may retry with the same `Idempotency-Key`.
    #[error("error communicating with gocardless: {0}")]
    ClientError(String),

    /// HTTP timeout. Same retry semantics as `ClientError`.
    #[error("timeout communicating with gocardless")]
    Timeout,

    /// GoCardless responded with a 4xx / 5xx body. The `RequestError`
    /// preserves `request_id` so we can echo it to support.
    #[error("error reported by gocardless: {0}")]
    Api(#[from] RequestError),

    /// Serialization / deserialization failures — programmer bug, not retryable.
    #[error("error encoding gocardless request")]
    Encode(#[from] serde_json::Error),
}

impl From<reqwest::Error> for GoCardlessError {
    fn from(err: reqwest::Error) -> GoCardlessError {
        if err.is_timeout() {
            GoCardlessError::Timeout
        } else {
            GoCardlessError::ClientError(err.to_string())
        }
    }
}

/// Wire shape of an error response. GoCardless documents:
/// `{ "error": { "type", "code", "message", "errors": [{...}], "request_id" } }`.
#[derive(Debug, Default, Deserialize, Error)]
#[error("{error_type:?} ({http_status}) request_id={request_id:?} message={message:?}")]
pub struct RequestError {
    #[serde(skip_deserializing)]
    pub http_status: u16,

    /// `validation_failed` | `invalid_api_usage` | `gocardless` | …
    #[serde(rename = "type")]
    pub error_type: Option<String>,

    /// Numeric code (e.g. 422 for validation).
    pub code: Option<i32>,

    pub message: Option<String>,

    /// Echoed in `Idempotent-Replayed`-style headers; the value support
    /// staff use to trace requests in GoCardless dashboards.
    pub request_id: Option<String>,

    #[serde(default)]
    pub errors: Vec<RequestErrorDetail>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RequestErrorDetail {
    pub field: Option<String>,
    pub message: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorResponse {
    pub error: RequestError,
}

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("invalid hmac key length")]
    BadKey,
    #[error("signature header missing or malformed")]
    BadSignature,
    #[error("error parsing event payload: {0}")]
    BadParse(#[from] serde_json::Error),
}
