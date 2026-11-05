use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PaymentRemindersApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("Not found: {0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for PaymentRemindersApiError {
    fn from(value: Report<StoreError>) -> Self {
        match value.current_context() {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::FailedPrecondition(msg) => Self::FailedPrecondition(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::NotFound(msg.clone()),
            _ => Self::StoreError(
                "Error in payment reminders service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
