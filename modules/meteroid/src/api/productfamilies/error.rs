use deadpool_postgres::tokio_postgres;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum ProductFamilyApiError {
    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] tokio_postgres::Error),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl Into<ProductFamilyApiError> for error_stack::Report<meteroid_store::errors::StoreError> {
    fn into(self) -> ProductFamilyApiError {
        let err = Box::new(self.into_error());
        ProductFamilyApiError::StoreError("Error in product_family service".to_string(), err)
    }
}
