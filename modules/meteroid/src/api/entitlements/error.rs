use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum EntitlementApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),
}

impl From<Report<StoreError>> for EntitlementApiError {
    fn from(err: Report<StoreError>) -> Self {
        match err.current_context() {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::DuplicateValue { entity, key } => Self::InvalidArgument(format!(
                "{entity} already exists{}",
                key.as_deref().map(|k| format!(": {k}")).unwrap_or_default()
            )),
            _ => Self::StoreError(
                "Error in entitlements service".to_string(),
                Box::new(err.into_error()),
            ),
        }
    }
}
