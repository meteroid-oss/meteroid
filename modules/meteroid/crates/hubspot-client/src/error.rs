use thiserror::Error;

#[derive(Debug, Error)]
pub enum HubspotError {
    #[error("error communicating with Hubspot: {error} {status_code:?}")]
    ClientError {
        error: String,
        status_code: Option<u16>,
    },
}

impl From<reqwest_middleware::Error> for HubspotError {
    fn from(err: reqwest_middleware::Error) -> HubspotError {
        HubspotError::ClientError {
            error: err.to_string(),
            status_code: None,
        }
    }
}

impl From<reqwest::Error> for HubspotError {
    fn from(err: reqwest::Error) -> HubspotError {
        HubspotError::ClientError {
            error: err.to_string(),
            status_code: None,
        }
    }
}
