use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum ProductApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl Into<ProductApiError> for error_stack::Report<meteroid_store::errors::StoreError> {
    fn into(self) -> ProductApiError {
        let err = Box::new(self.into_error());
        ProductApiError::StoreError("Error in product service".to_string(), err)
    }
}
