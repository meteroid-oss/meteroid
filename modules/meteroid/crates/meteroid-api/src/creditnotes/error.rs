use std::error::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use error_stack::Report;
use meteroid_app::errors::InvoicingRenderError;
use meteroid_store::errors::StoreError;
use thiserror::Error;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum CreditNoteApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Credit note not found")]
    #[code(NotFound)]
    CreditNoteNotFound,

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("Input error: {0}")]
    #[code(InvalidArgument)]
    InputError(String),

    #[error("Rendering error: {0}")]
    #[code(Internal)]
    RenderingError(String),
}

impl From<Report<StoreError>> for CreditNoteApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = value.current_context();

        match err {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::FailedPrecondition(msg) => Self::FailedPrecondition(msg.clone()),
            StoreError::Conflict(msg) => Self::FailedPrecondition(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::DuplicateValue { entity, key } => {
                let msg = match key {
                    Some(k) => format!("{} with key '{}' already exists", entity, k),
                    None => format!("{} already exists", entity),
                };
                Self::InvalidArgument(msg)
            }
            _ => Self::StoreError(
                "Error in credit note service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}

impl From<Report<InvoicingRenderError>> for CreditNoteApiError {
    fn from(value: Report<InvoicingRenderError>) -> Self {
        Self::RenderingError(format!("{:?}", value))
    }
}
