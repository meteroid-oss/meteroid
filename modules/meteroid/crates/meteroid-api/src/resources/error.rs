use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use error_stack::Report;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::resources::ResourceRefusal;
use std::error::Error;
use thiserror::Error;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum ResourcesApiError {
    #[error("{0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("{0}")]
    #[code(AlreadyExists)]
    AlreadyExists(String),

    #[error("{0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

/// A coded refusal reads `<CODE>: <message>`.
impl From<Report<StoreError>> for ResourcesApiError {
    fn from(value: Report<StoreError>) -> Self {
        let refusal = ResourceRefusal::of(&value);
        let coded = |message: &String| match refusal {
            Some(refusal) => format!("{}: {message}", refusal.code()),
            None => message.clone(),
        };
        match value.current_context() {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(coded(msg)),
            StoreError::Conflict(msg) => Self::AlreadyExists(coded(msg)),
            StoreError::DuplicateValue { entity, key } => Self::AlreadyExists(match key {
                Some(key) => format!("{entity} `{key}` already exists"),
                None => format!("{entity} already exists"),
            }),
            StoreError::ValueNotFound(msg) => Self::NotFound(msg.clone()),
            StoreError::FailedPrecondition(msg) => Self::FailedPrecondition(msg.clone()),
            _ => Self::StoreError(
                "Error in resources service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
