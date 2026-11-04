//! What the REST writes share: the Idempotency-Key header, the request fingerprint and how a
//! store refusal reads.
use crate::errors::RestApiError;
use error_stack::Report;
use http::HeaderMap;
use meteroid_store::domain::credit::ATTESTATION_REQUIRED;
use meteroid_store::errors::{RateLimited, StoreError};
use sha2::{Digest, Sha256};

const IDEMPOTENCY_KEY: &str = "Idempotency-Key";

pub fn invalid(message: impl Into<String>) -> RestApiError {
    RestApiError::InvalidInput(message.into())
}

pub fn idempotency_key(headers: &HeaderMap) -> Result<&str, RestApiError> {
    headers
        .get(IDEMPOTENCY_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| invalid("the Idempotency-Key header is required"))
}

/// Only the request itself is fingerprinted, never the time or a resolved default.
pub fn fingerprint(request: &impl serde::Serialize) -> Result<[u8; 32], RestApiError> {
    let bytes = serde_json::to_vec(request).map_err(|error| invalid(error.to_string()))?;
    Ok(Sha256::digest(bytes).into())
}

/// Answers 409 when the customer's state refuses the request.
pub fn store_error(error: Report<StoreError>) -> RestApiError {
    if let Some(RateLimited(message)) = error
        .frames()
        .find_map(|frame| frame.downcast_ref::<RateLimited>())
    {
        return RestApiError::RateLimitExceeded(message.clone());
    }
    match error.current_context() {
        StoreError::Conflict(message) | StoreError::FailedPrecondition(message) => {
            RestApiError::FailedPrecondition(message.clone())
        }
        StoreError::InvalidArgument(message) if message == ATTESTATION_REQUIRED => {
            RestApiError::InvalidInput(format!("{message} Set `customer_consent_attested`."))
        }
        _ => error.into(),
    }
}
