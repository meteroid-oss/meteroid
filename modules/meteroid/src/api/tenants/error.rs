use deadpool_postgres::tokio_postgres;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum TenantApiError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Downstream service error: {0}")]
    #[code(Internal)]
    DownstreamApiError(String, #[source] Box<dyn std::error::Error + Sync + Send>),

    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] tokio_postgres::Error),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl Into<TenantApiError> for error_stack::Report<meteroid_store::errors::StoreError> {
    fn into(self) -> TenantApiError {
        let err = Box::new(self.into_error());
        TenantApiError::StoreError("Error in tenant service".to_string(), err)
    }
}
