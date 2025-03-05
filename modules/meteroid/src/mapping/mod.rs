use meteroid_store::errors::StoreError;
use tonic::Status;

pub mod common;

#[derive(Debug, thiserror::Error)]
#[error("MappingError: {message}")]
pub struct MappingError {
    pub message: String,
}

impl MappingError {
    pub fn new<S: ToString>(message: S) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

impl From<MappingError> for Status {
    fn from(error: MappingError) -> Self {
        Status::new(tonic::Code::InvalidArgument, error.message)
    }
}

impl From<MappingError> for StoreError {
    fn from(error: MappingError) -> Self {
        StoreError::InvalidArgument(error.message)
    }
}
