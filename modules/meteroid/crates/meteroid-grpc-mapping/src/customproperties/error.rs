use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum CustomPropertiesApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("{0}")]
    #[code(AlreadyExists)]
    AlreadyExists(String),

    #[error("Not found: {0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for CustomPropertiesApiError {
    fn from(value: Report<StoreError>) -> Self {
        match value.current_context() {
            StoreError::DuplicateValue { key, .. } => Self::AlreadyExists(match key {
                Some(key) => format!("A custom property with key '{key}' already exists"),
                None => "A custom property with that key already exists".to_string(),
            }),
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::NotFound(msg.clone()),
            _ => Self::StoreError(
                "Error in custom properties service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
