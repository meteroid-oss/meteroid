use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use crate::errors::InvoicingRenderError;
use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
#[allow(clippy::enum_variant_names)]
pub enum InvoiceApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
    #[error("Render error: {0}")]
    #[code(Internal)]
    RenderError(String, #[source] Box<dyn Error>),
    #[error("Input error: {0}")]
    #[code(Internal)]
    InputError(String),
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),
}

impl From<Report<StoreError>> for InvoiceApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = value.current_context();

        match err {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::DuplicateValue { entity, key } => {
                let msg = match key {
                    Some(k) => format!("{} with key '{}' already exists", entity, k),
                    None => format!("{} already exists", entity),
                };
                Self::InvalidArgument(msg)
            }
            _ => Self::StoreError(
                "Error in invoice service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}

impl From<Report<InvoicingRenderError>> for InvoiceApiError {
    fn from(value: Report<InvoicingRenderError>) -> Self {
        let err = Box::new(value.into_error());
        Self::RenderError("Error in invoice service".to_string(), err)
    }
}
