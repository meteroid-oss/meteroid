use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use crate::errors::ObjectStoreError;
use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PortalSharedApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
    #[error("Object store error: {0}")]
    #[code(Internal)]
    ObjectStoreError(String, #[source] Box<dyn Error>),
    #[error("Subscription has no configured payment provider")]
    #[code(InvalidArgument)]
    MissingCustomerConnection,
    #[error("Failed to update customer")]
    #[code(Internal)]
    CustomerUpdateError,
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),
}

impl From<Report<StoreError>> for PortalSharedApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::StoreError("Error in portal checkout service".to_string(), err)
    }
}
impl From<Report<ObjectStoreError>> for PortalSharedApiError {
    fn from(value: Report<ObjectStoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::ObjectStoreError(
            "Object store error in portal checkout service".to_string(),
            err,
        )
    }
}
