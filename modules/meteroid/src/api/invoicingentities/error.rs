use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use crate::errors::ObjectStoreError;
use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum InvoicingEntitiesApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("Object store error: {0}")]
    #[code(Internal)]
    ObjectStoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for InvoicingEntitiesApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = Box::new(value.into_error());
        InvoicingEntitiesApiError::StoreError(
            "Error in invoicing entities service".to_string(),
            err,
        )
    }
}
impl From<Report<ObjectStoreError>> for InvoicingEntitiesApiError {
    fn from(value: Report<ObjectStoreError>) -> Self {
        let err = Box::new(value.into_error());
        InvoicingEntitiesApiError::ObjectStoreError(
            "Error with object store in invoicing entities service".to_string(),
            err,
        )
    }
}
