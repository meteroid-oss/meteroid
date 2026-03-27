use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use error_stack::Report;
use meteroid_store::errors::StoreError;
use std::error::Error;
use thiserror::Error;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum BatchJobApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Object store error: {0}")]
    #[code(Internal)]
    ObjectStoreError(String),

    #[error("Duplicate: {0}")]
    #[code(AlreadyExists)]
    DuplicateImport(String),
}

impl From<Report<StoreError>> for BatchJobApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::StoreError("Error in batch job service".to_string(), err)
    }
}
