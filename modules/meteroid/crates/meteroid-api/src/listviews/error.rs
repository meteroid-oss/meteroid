use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum ListViewsApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("{0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("{0}")]
    #[code(PermissionDenied)]
    PermissionDenied(String),

    #[error("{0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for ListViewsApiError {
    fn from(value: Report<StoreError>) -> Self {
        match value.current_context() {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(_) => Self::NotFound("View not found".to_string()),
            StoreError::Forbidden(msg) => Self::PermissionDenied(msg.clone()),
            _ => Self::StoreError(
                "Error in list views service".to_string(),
                Box::new(value.attach("Error in api list view service").into_error()),
            ),
        }
    }
}
