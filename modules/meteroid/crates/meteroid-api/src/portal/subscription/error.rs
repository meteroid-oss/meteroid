use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PortalSubscriptionApiError {
    #[error("{}", crate::portal::INTERNAL_ERROR)]
    #[code(Internal)]
    StoreError(#[source] Box<dyn Error>),
    #[error("{0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),
    #[error("{0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),
    #[error("{0}")]
    #[code(NotFound)]
    NotFound(String),
    #[error("Unauthorized")]
    #[code(PermissionDenied)]
    Unauthorized,
    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),
}

impl From<Report<StoreError>> for PortalSubscriptionApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = value.current_context();

        match err {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::NotFound(msg.clone()),
            _ => Self::StoreError(Box::new(value.into_error())),
        }
    }
}
