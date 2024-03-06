use deadpool_postgres::tokio_postgres;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum WebhookServiceError {
    #[error("Unknown error occurred: {0}")]
    #[code(InvalidArgument)]
    UnknownError(String),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Serialization error: {0}")]
    #[code(InvalidArgument)]
    SerializationError(String, #[source] serde_json::Error),

    #[error("Mapping error: {0}")]
    #[code(Internal)]
    MappingError(
        String,
        #[source] crate::api::services::errors::DatabaseError,
    ),

    #[error("Entity not found: {0}")]
    #[code(NotFound)]
    DatabaseEntityNotFoundError(String),

    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] tokio_postgres::Error),
}
