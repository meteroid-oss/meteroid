use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum CustomerApiError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Serialization error: {0}")]
    #[code(InvalidArgument)]
    SerializationError(String, #[source] serde_json::Error),

    #[error("Mapping error: {0}")]
    #[code(Internal)]
    MappingError(String, #[source] crate::api::errors::DatabaseError),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl Into<CustomerApiError> for error_stack::Report<meteroid_store::errors::StoreError> {
    fn into(self) -> CustomerApiError {
        let err = Box::new(self.into_error());
        CustomerApiError::StoreError("Error in tenant service".to_string(), err)
    }
}
