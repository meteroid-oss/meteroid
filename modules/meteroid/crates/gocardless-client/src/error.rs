//! `Timeout` / `ClientError` / `Api` partitioning lets the connector adapter route
//! transport failures (retryable) separately from logical API failures.

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

/// Client-side failure surfaced before any HTTP call (bad idempotency key,
/// metadata over GoCardless limits). Reuses `Encode` so it stays non-retryable
/// without adding a variant the store adapter's exhaustive match can't see.
pub(crate) fn client_validation(msg: impl Into<String>) -> GoCardlessError {
    GoCardlessError::Encode(<serde_json::Error as serde::ser::Error>::custom(msg.into()))
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

    /// Used to trace requests in GoCardless dashboards / with support.
    pub request_id: Option<String>,

    #[serde(default)]
    pub errors: Vec<RequestErrorDetail>,
}

impl RequestError {
    /// GoCardless 409 on idempotent replay: unlike Stripe it does NOT return the
    /// original 2xx — it answers `type=invalid_state` with a detail whose
    /// `reason=idempotent_creation_conflict` pointing at the first attempt's id.
    pub fn is_idempotent_creation_conflict(&self) -> bool {
        self.http_status == 409
            && self
                .errors
                .iter()
                .any(|e| e.reason.as_deref() == Some("idempotent_creation_conflict"))
    }

    /// Id of the resource the first (successful) attempt created, if present.
    pub fn conflicting_resource_id(&self) -> Option<&str> {
        self.errors
            .iter()
            .find_map(|e| e.links.conflicting_resource_id.as_deref())
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct RequestErrorDetail {
    pub field: Option<String>,
    pub message: Option<String>,
    pub reason: Option<String>,
    #[serde(default, deserialize_with = "crate::null_as_default")]
    pub links: RequestErrorLinks,
}

/// Per-error links. On `idempotent_creation_conflict`, `conflicting_resource_id`
/// carries the id created by the original request.
#[derive(Debug, Default, Deserialize)]
pub struct RequestErrorLinks {
    pub conflicting_resource_id: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-world shape of the GoCardless idempotent-conflict 409 body.
    const IDEMPOTENT_CONFLICT_BODY: &str = r#"{
        "error": {
            "type": "invalid_state",
            "code": 409,
            "message": "A resource has already been created with this idempotency key",
            "request_id": "req_123",
            "errors": [{
                "reason": "idempotent_creation_conflict",
                "message": "A resource has already been created with this idempotency key",
                "links": { "conflicting_resource_id": "PM123456789" }
            }]
        }
    }"#;

    fn parse(body: &str, status: u16) -> RequestError {
        let mut err = serde_json::from_str::<ErrorResponse>(body)
            .expect("error body should parse")
            .error;
        err.http_status = status;
        err
    }

    #[test]
    fn parses_idempotent_conflict_and_extracts_conflicting_id() {
        let err = parse(IDEMPOTENT_CONFLICT_BODY, 409);
        assert_eq!(err.error_type.as_deref(), Some("invalid_state"));
        assert!(err.is_idempotent_creation_conflict());
        assert_eq!(err.conflicting_resource_id(), Some("PM123456789"));
    }

    #[test]
    fn other_409_is_not_idempotent_conflict() {
        let body = r#"{
            "error": {
                "type": "invalid_state",
                "code": 409,
                "message": "Mandate is not active",
                "errors": [{ "reason": "mandate_is_inactive" }]
            }
        }"#;
        let err = parse(body, 409);
        assert!(!err.is_idempotent_creation_conflict());
        assert_eq!(err.conflicting_resource_id(), None);
    }

    #[test]
    fn non_409_with_conflict_reason_does_not_trigger() {
        // Same reason string but a 422 status must not be treated as a conflict.
        let err = parse(IDEMPOTENT_CONFLICT_BODY, 422);
        assert!(!err.is_idempotent_creation_conflict());
    }

    #[test]
    fn validation_failed_422_is_not_conflict() {
        let body = r#"{
            "error": {
                "type": "validation_failed",
                "code": 422,
                "message": "Validation failed",
                "errors": [{ "field": "amount", "message": "is required" }]
            }
        }"#;
        let err = parse(body, 422);
        assert!(!err.is_idempotent_creation_conflict());
        assert_eq!(err.conflicting_resource_id(), None);
    }
}
