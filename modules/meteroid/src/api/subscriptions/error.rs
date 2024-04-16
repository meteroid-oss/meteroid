use deadpool_postgres::tokio_postgres;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum SubscriptionApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Calculation error: {0}")]
    #[code(Internal)]
    CalculationError(String, #[source] anyhow::Error),

    #[error("Serialization error: {0}")]
    #[code(Internal)]
    SerializationError(String, #[source] serde_json::Error),

    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] tokio_postgres::Error),
}
