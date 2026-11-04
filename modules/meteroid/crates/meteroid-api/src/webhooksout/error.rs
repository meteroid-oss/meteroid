use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum WebhookApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

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

impl From<Report<StoreError>> for WebhookApiError {
    fn from(value: Report<StoreError>) -> Self {
        match value.current_context() {
            StoreError::ValueNotFound(msg) => return Self::NotFound(msg.clone()),
            StoreError::FailedPrecondition(msg) => return Self::FailedPrecondition(msg.clone()),
            _ => {}
        }

        let message = value.current_context().to_string();
        let err = Box::new(
            value
                .attach("Error in api webhooks out service".to_string())
                .into_error(),
        );

        Self::StoreError(message, err)
    }
}
