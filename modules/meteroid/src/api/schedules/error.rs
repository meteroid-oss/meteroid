use deadpool_postgres::tokio_postgres;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum ScheduleApiError {
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
    StoreError(String, #[source] Box<dyn Error>),
}

impl Into<ScheduleApiError> for error_stack::Report<meteroid_store::errors::StoreError> {
    fn into(self) -> ScheduleApiError {
        let err = self.current_context();

        match err {
            meteroid_store::errors::StoreError::InvalidArgument(str) => {
                ScheduleApiError::InvalidArgument(str.clone())
            }
            _e => ScheduleApiError::StoreError(
                "Error in schedule service".to_string(),
                Box::new(self.into_error()),
            ),
        }
    }
}
