use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use errors::StoreError;
use meteroid_store::compute::ComputeError;
use meteroid_store::errors;

#[derive(Debug, Error, ErrorAsTonic)]
#[allow(clippy::enum_variant_names)]
pub enum BillableMetricApiError {
    #[error("Mapping error: {0}")]
    #[code(Internal)]
    MappingError(String, #[source] prost::DecodeError),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("Metering Service error: {0}")]
    #[code(Internal)]
    MeteringServiceError(String, #[source] ComputeError),
}

impl From<Report<StoreError>> for BillableMetricApiError {
    fn from(err: Report<StoreError>) -> Self {
        Self::StoreError(
            "Error in billable metric service".to_string(),
            Box::new(err.into_error()),
        )
    }
}
