use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use error_stack::Report;
use meteroid_store::errors::StoreError;
use std::error::Error;
use thiserror::Error;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum DeadLetterApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Not found: {0}")]
    #[code(NotFound)]
    NotFound(String),
}

impl From<Report<StoreError>> for DeadLetterApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::StoreError("Error in dead letter service".to_string(), err)
    }
}
