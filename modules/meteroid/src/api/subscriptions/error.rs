use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum SubscriptionApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Calculation error: {0}")]
    #[code(Internal)]
    CalculationError(String, #[source] crate::compute::ComputeError),

    #[error("Failed to retrieve the subscription details: {0}")]
    #[code(Internal)]
    SubscriptionDetailsError(String, #[source] anyhow::Error),

    #[error("Serialization error: {0}")]
    #[code(Internal)]
    SerializationError(String, #[source] serde_json::Error),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for SubscriptionApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::StoreError("Error in subscription service".to_string(), err)
    }
}
