use std::error::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use deadpool_postgres::PoolError;
use error_stack::Report;
use meteroid_store::errors::StoreError;
use thiserror::Error;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum CreditNoteApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Credit note not found")]
    #[code(NotFound)]
    CreditNoteNotFound,

    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] Option<Box<dyn Error + Send + Sync>>),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),

    #[error("Input error: {0}")]
    #[code(InvalidArgument)]
    InputError(String),
}

impl From<Report<StoreError>> for CreditNoteApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::StoreError("Error in credit note service".to_string(), err)
    }
}

impl From<PoolError> for CreditNoteApiError {
    fn from(e: PoolError) -> Self {
        CreditNoteApiError::DatabaseError(e.to_string(), Some(Box::new(e)))
    }
}
