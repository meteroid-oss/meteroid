use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum ScheduleApiError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for ScheduleApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = value.current_context();

        match err {
            StoreError::InvalidArgument(str) => Self::InvalidArgument(str.clone()),
            _e => Self::StoreError(
                "Error in schedule service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
