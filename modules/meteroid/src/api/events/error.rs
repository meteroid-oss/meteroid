use meteroid_store::errors::StoreError;
use thiserror::Error;
use tonic::{Code, Status};

#[derive(Debug, Error)]
pub enum EventsApiError {
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("CSV parsing error: {0}")]
    CsvParsingError(String),

    #[error("Store error: {0}")]
    StoreError(#[from] StoreError),

    #[error("Metering service error: {0}")]
    MeteringServiceError(String),
}

impl From<EventsApiError> for Status {
    fn from(error: EventsApiError) -> Self {
        match error {
            EventsApiError::InvalidArgument(msg) => Status::new(Code::InvalidArgument, msg),
            EventsApiError::CsvParsingError(msg) => {
                Status::new(Code::InvalidArgument, format!("CSV parsing error: {}", msg))
            }
            EventsApiError::StoreError(err) => match err {
                _ => Status::new(Code::Internal, format!("Store error: {}", err)),
            },
            EventsApiError::MeteringServiceError(msg) => {
                Status::new(Code::Internal, format!("Metering service error: {}", msg))
            }
        }
    }
}
