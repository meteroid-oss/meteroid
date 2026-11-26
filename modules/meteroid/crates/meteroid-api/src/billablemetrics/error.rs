use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use errors::StoreError;
use meteroid_store::errors;

#[derive(Debug, Error, ErrorAsTonic)]
#[allow(clippy::enum_variant_names)]
pub enum BillableMetricApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Mapping error: {0}")]
    #[code(Internal)]
    MappingError(String, #[source] prost::DecodeError),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for BillableMetricApiError {
    fn from(err: Report<StoreError>) -> Self {
        match err.current_context() {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::FailedPrecondition(msg) => Self::FailedPrecondition(msg.clone()),
            _ => Self::StoreError(
                "Error in billable metric service".to_string(),
                Box::new(err.into_error()),
            ),
        }
    }
}
