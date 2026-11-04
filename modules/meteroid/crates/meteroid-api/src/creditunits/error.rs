use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum CreditUnitsApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("{0}")]
    #[code(AlreadyExists)]
    AlreadyExists(String),

    #[error("Not found: {0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for CreditUnitsApiError {
    fn from(value: Report<StoreError>) -> Self {
        match value.current_context() {
            StoreError::DuplicateValue { key, .. } => Self::AlreadyExists(match key {
                Some(code) => format!("A credit unit with code '{code}' already exists"),
                None => "A credit unit with that code already exists".to_string(),
            }),
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::NotFound(msg.clone()),
            StoreError::FailedPrecondition(msg) => Self::FailedPrecondition(msg.clone()),
            _ => Self::StoreError(
                "Error in credit units service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
