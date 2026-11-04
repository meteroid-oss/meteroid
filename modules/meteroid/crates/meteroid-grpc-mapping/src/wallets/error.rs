use std::error::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use error_stack::Report;
use meteroid_store::errors::{RateLimited, StoreError};
use thiserror::Error;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum WalletApiError {
    #[error("{0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("{0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("{0}")]
    #[code(PermissionDenied)]
    PermissionDenied(String),

    #[error("{0}")]
    #[code(Aborted)]
    Conflict(String),

    #[error("{0}")]
    #[code(ResourceExhausted)]
    RateLimited(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for WalletApiError {
    fn from(value: Report<StoreError>) -> Self {
        if let Some(RateLimited(message)) = value
            .frames()
            .find_map(|frame| frame.downcast_ref::<RateLimited>())
        {
            return Self::RateLimited(message.clone());
        }
        match value.current_context() {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::NotFound(format!("{msg} not found")),
            StoreError::FailedPrecondition(msg) => Self::FailedPrecondition(msg.clone()),
            StoreError::Conflict(msg) => Self::Conflict(msg.clone()),
            StoreError::Forbidden(msg) => Self::PermissionDenied(msg.clone()),
            _ => Self::StoreError(
                "Error in wallet service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
