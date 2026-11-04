use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum OrganizationApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("{0}")]
    #[code(PermissionDenied)]
    PermissionDenied(String),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for OrganizationApiError {
    fn from(value: Report<StoreError>) -> Self {
        match value.current_context() {
            StoreError::Forbidden(msg) => OrganizationApiError::PermissionDenied(msg.clone()),
            StoreError::FailedPrecondition(msg) => {
                OrganizationApiError::FailedPrecondition(msg.clone())
            }
            _ => OrganizationApiError::StoreError(
                "Error in organization service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
