use thiserror::Error;

#[derive(Debug, Error)]
pub enum PennylaneError {
    #[error("error communicating with Pennylane: {error} {status_code:?}")]
    ClientError {
        error: String,
        status_code: Option<u16>,
    },
}

impl PennylaneError {
    pub fn status_code(&self) -> Option<u16> {
        match self {
            PennylaneError::ClientError { status_code, .. } => *status_code,
        }
    }
}

impl From<reqwest_middleware::Error> for PennylaneError {
    fn from(err: reqwest_middleware::Error) -> PennylaneError {
        PennylaneError::ClientError {
            error: err.to_string(),
            status_code: None,
        }
    }
}

impl From<reqwest::Error> for PennylaneError {
    fn from(err: reqwest::Error) -> PennylaneError {
        PennylaneError::ClientError {
            error: err.to_string(),
            status_code: None,
        }
    }
}
