use std::error::Error;
use deadpool_postgres::tokio_postgres;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PriceComponentApiError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Serialization error: {0}")]
    #[code(InvalidArgument)]
    SerializationError(String, #[source] serde_json::Error),

    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] tokio_postgres::Error),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(
        String,
        #[source] Box<dyn Error>,
    ),
}

impl Into<PriceComponentApiError> for error_stack::Report<meteroid_store::errors::StoreError> {
    fn into(self) -> PriceComponentApiError {
        let err = Box::new(self.into_error());
        PriceComponentApiError::StoreError("Error in price component service".to_string(), err)
    }
}
