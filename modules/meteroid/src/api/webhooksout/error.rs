use deadpool_postgres::tokio_postgres;
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
}
