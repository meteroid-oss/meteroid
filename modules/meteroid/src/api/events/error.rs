use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum EventsApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("CSV parsing error: {0}")]
    #[code(InvalidArgument)]
    CsvParsingError(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("Metering service error: {0}")]
    #[code(Internal)]
    MeteringServiceError(String),
}

impl From<Report<StoreError>> for EventsApiError {
    fn from(value: Report<StoreError>) -> Self {
        Self::StoreError(
            "Error in events service".to_string(),
            Box::new(value.into_error()),
        )
    }
}
