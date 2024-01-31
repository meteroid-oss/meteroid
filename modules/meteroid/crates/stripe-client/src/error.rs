use serde::Deserialize;
use std::num::ParseIntError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("invalid key length")]
    BadKey,
    #[error("error parsing timestamp")]
    BadHeader(#[from] ParseIntError),
    #[error("error comparing signatures")]
    BadSignature,
    #[error("error comparing timestamps - over tolerance")]
    BadTimestamp(i64),
    #[error("error parsing event object")]
    BadParse(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum StripeError {
    #[error("error reported by stripe: {0}")]
    Stripe(#[from] RequestError),
    #[error("error serializing or deserializing a querystring: {0}")]
    QueryStringSerialize(#[from] serde_path_to_error::Error<serde_qs::Error>),
    #[error("error serializing or deserializing a request")]
    JSONSerialize(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error("attempted to access an unsupported version of the api")]
    UnsupportedVersion,
    #[error("error communicating with stripe: {0}")]
    ClientError(String),
    #[error("timeout communicating with stripe")]
    Timeout,
}

/// An error reported by stripe in a request's response.
///
/// For more details see <https://stripe.com/docs/api#errors>.
#[derive(Debug, Default, Deserialize, Error)]
#[error("{error_type} ({http_status}) with message: {message:?}")]
pub struct RequestError {
    /// The HTTP status in the response.
    #[serde(skip_deserializing)]
    pub http_status: u16,

    /// The type of error returned.
    #[serde(rename = "type")]
    pub error_type: String,

    /// A human-readable message providing more details about the error.
    /// For card errors, these messages can be shown to end users.
    #[serde(default)]
    pub message: Option<String>,

    /// For card errors, a value describing the kind of card error that occurred.
    pub code: Option<String>,
}

/// The structure of the json body when an error is included in
/// the response from Stripe.
#[derive(Deserialize)]
pub struct ErrorResponse {
    pub error: RequestError,
}

impl From<reqwest::Error> for StripeError {
    fn from(err: reqwest::Error) -> StripeError {
        StripeError::ClientError(err.to_string())
    }
}
