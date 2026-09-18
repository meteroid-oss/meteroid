use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MollieError {
    #[error("error reported by mollie: {0}")]
    Mollie(#[from] RequestError),
    #[error("error serializing or deserializing a request")]
    JSONSerialize(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error("error communicating with mollie: {0}")]
    ClientError(String),
}

/// `error-response` schema; `field` names the offending request field on 422s.
#[derive(Debug, Default, Deserialize, Error)]
#[error("{}{}", detail, field.as_deref().map(|f| format!(" (field: {f})")).unwrap_or_default())]
pub struct RequestError {
    #[serde(default)]
    pub status: u16,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub field: Option<String>,
}

impl RequestError {
    pub fn http_status(&self) -> u16 {
        self.status
    }
}

impl From<reqwest::Error> for MollieError {
    fn from(err: reqwest::Error) -> MollieError {
        MollieError::ClientError(err.to_string())
    }
}
