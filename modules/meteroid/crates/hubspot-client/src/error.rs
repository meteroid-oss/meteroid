use thiserror::Error;

#[derive(Debug, Error)]
pub enum HubspotError {
    #[error("error communicating with Hubspot: {0}")]
    ClientError(String),
}

impl From<reqwest_middleware::Error> for HubspotError {
    fn from(err: reqwest_middleware::Error) -> HubspotError {
        HubspotError::ClientError(err.to_string())
    }
}

impl From<reqwest::Error> for HubspotError {
    fn from(err: reqwest::Error) -> HubspotError {
        HubspotError::ClientError(err.to_string())
    }
}
