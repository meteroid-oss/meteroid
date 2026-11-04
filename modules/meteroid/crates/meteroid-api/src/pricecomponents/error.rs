use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PriceComponentApiError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("Not found: {0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("Serialization error: {0}")]
    #[code(InvalidArgument)]
    SerializationError(String, #[source] serde_json::Error),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for PriceComponentApiError {
    fn from(value: Report<StoreError>) -> Self {
        match value.current_context() {
            StoreError::FailedPrecondition(msg) => return Self::FailedPrecondition(msg.clone()),
            StoreError::ValueNotFound(msg) => return Self::NotFound(msg.clone()),
            _ => {}
        }
        let err = Box::new(value.into_error());
        Self::StoreError("Error in api price component service".to_string(), err)
    }
}
