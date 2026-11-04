use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum TenantApiError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("{0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("{0}")]
    #[code(PermissionDenied)]
    PermissionDenied(String),
}

impl From<Report<StoreError>> for TenantApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = value.current_context();
        match err {
            StoreError::FailedPrecondition(msg) => Self::FailedPrecondition(msg.clone()),
            StoreError::Forbidden(msg) => Self::PermissionDenied(msg.clone()),
            _ => Self::StoreError(
                "Error in tenant service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
