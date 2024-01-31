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
