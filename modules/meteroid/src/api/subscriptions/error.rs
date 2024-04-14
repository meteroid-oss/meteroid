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
    CalculationError(String, #[source] crate::compute2::ComputeError),

    #[error("Failed to retrieve the subscription details: {0}")]
    #[code(Internal)]
    SubscriptionDetailsError(String, #[source] anyhow::Error),

    #[error("Serialization error: {0}")]
    #[code(Internal)]
    SerializationError(String, #[source] serde_json::Error),

    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] tokio_postgres::Error),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(
        String,
        error_stack::Report<meteroid_store::errors::StoreError>,
    ),
}

impl Into<SubscriptionApiError> for error_stack::Report<meteroid_store::errors::StoreError> {
    fn into(self) -> SubscriptionApiError {
        SubscriptionApiError::StoreError("Error in subscription service".to_string(), self)
    }
}
