use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StancerError {
    #[error("error reported by stancer: {0}")]
    Stancer(#[from] RequestError),
    #[error("error serializing or deserializing a request")]
    JSONSerialize(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error("error communicating with stancer: {0}")]
    ClientError(String),
}

/// A validation error reported by Stancer's API (FastAPI-style `422` body).
///
/// Verified live shape: `{"detail":[{"loc":["body","auth"],"msg":"Auth can't
/// be False","type":"value_error"}]}`.
#[derive(Debug, Default, Deserialize, Error)]
#[error("{}", format_detail(detail))]
pub struct RequestError {
    #[serde(skip_deserializing)]
    pub http_status: u16,

    #[serde(default)]
    pub detail: Vec<ValidationErrorItem>,
}

fn format_detail(detail: &[ValidationErrorItem]) -> String {
    detail
        .iter()
        .map(|d| format!("{}: {}", d.loc.join("."), d.msg))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct ValidationErrorItem {
    #[serde(default)]
    pub loc: Vec<String>,
    pub msg: String,
    #[serde(rename = "type")]
    pub error_type: String,
}

impl From<reqwest::Error> for StancerError {
    fn from(err: reqwest::Error) -> StancerError {
        StancerError::ClientError(err.to_string())
    }
}
