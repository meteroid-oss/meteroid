use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum RbacApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Permission denied: {0}")]
    #[code(PermissionDenied)]
    PermissionDenied(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for RbacApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = value.current_context();
        match err {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::DuplicateValue { .. } => {
                Self::InvalidArgument("A role with this name already exists".to_string())
            }
            _ => Self::StoreError(
                "Error in rbac service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
