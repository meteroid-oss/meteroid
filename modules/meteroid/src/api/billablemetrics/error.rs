use std::error::Error;

use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum BillableMetricApiError {
    #[error("Mapping error: {0}")]
    #[code(Internal)]
    MappingError(String, #[source] prost::DecodeError),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<error_stack::Report<meteroid_store::errors::StoreError>> for BillableMetricApiError {
    fn from(err: error_stack::Report<meteroid_store::errors::StoreError>) -> Self {
        Self::StoreError(
            "Error in billable metric service".to_string(),
            Box::new(err.into_error()),
        )
    }
}
