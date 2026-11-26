use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::{EntitlementRefusal, StoreError};
use meteroid_store::repositories::resources::ResourceRefusal;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum EntitlementApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Not found: {0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),
}

impl From<Report<StoreError>> for EntitlementApiError {
    fn from(err: Report<StoreError>) -> Self {
        let code = EntitlementRefusal::of(&err)
            .map(EntitlementRefusal::code)
            .or_else(|| ResourceRefusal::of(&err).map(ResourceRefusal::code));
        // A coded refusal reads `<CODE>: <message>`, as the resources service's.
        let coded = |message: &String| match code {
            Some(code) => format!("{code}: {message}"),
            None => message.clone(),
        };
        match err.current_context() {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(coded(msg)),
            StoreError::ValueNotFound(msg) => Self::NotFound(msg.clone()),
            StoreError::FailedPrecondition(msg) | StoreError::Conflict(msg) => {
                Self::FailedPrecondition(coded(msg))
            }
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
