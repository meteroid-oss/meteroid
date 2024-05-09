use deadpool_postgres::tokio_postgres;
use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum TenantApiError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Downstream service error: {0}")]
    #[code(Internal)]
    DownstreamApiError(String, #[source] Box<dyn std::error::Error + Sync + Send>),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for TenantApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = Box::new(value.into_error());
        TenantApiError::StoreError("Error in tenant service".to_string(), err)
    }
}
