use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PortalSubscriptionApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),
    #[error("Not found: {0}")]
    #[code(NotFound)]
    NotFound(String),
    #[error("Unauthorized")]
    #[code(PermissionDenied)]
    Unauthorized,
}

impl From<Report<StoreError>> for PortalSubscriptionApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = value.current_context();

        match err {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::NotFound(msg.clone()),
            _ => Self::StoreError(
                "Error in portal subscription service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
