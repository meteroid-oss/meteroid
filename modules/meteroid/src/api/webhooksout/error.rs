use deadpool_postgres::tokio_postgres;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum WebhookApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Entity not found: {0}")]
    #[code(NotFound)]
    DatabaseEntityNotFoundError(String),

    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] tokio_postgres::Error),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl Into<WebhookApiError> for error_stack::Report<meteroid_store::errors::StoreError> {
    fn into(self) -> WebhookApiError {
        let err = Box::new(self.into_error());
        WebhookApiError::StoreError("Error in tenant service".to_string(), err)
    }
}
