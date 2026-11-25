use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PlanApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Not found: {0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for PlanApiError {
    fn from(value: Report<StoreError>) -> Self {
        match value.current_context() {
            StoreError::InvalidArgument(message) => {
                return PlanApiError::InvalidArgument(message.clone());
            }
            // A version a plan doesn't have (a draft already published, an active one not yet).
            StoreError::ValueNotFound(message) => return PlanApiError::NotFound(message.clone()),
            _ => {}
        }
        let err = Box::new(value.into_error());
        PlanApiError::StoreError("Error in plan service".to_string(), err)
    }
}
