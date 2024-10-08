use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use crate::errors::InvoicingRenderError;
use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum InvoiceApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
    #[error("Render error: {0}")]
    #[code(Internal)]
    RenderError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for InvoiceApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::StoreError("Error in invoice service".to_string(), err)
    }
}

impl From<Report<InvoicingRenderError>> for InvoiceApiError {
    fn from(value: Report<InvoicingRenderError>) -> Self {
        let err = Box::new(value.into_error());
        Self::RenderError("Error in invoice service".to_string(), err)
    }
}
