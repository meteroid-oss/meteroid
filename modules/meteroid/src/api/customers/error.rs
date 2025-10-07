use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum CustomerApiError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Serialization error: {0}")]
    #[code(InvalidArgument)]
    SerializationError(String, #[source] serde_json::Error),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Mapping error: {0}")]
    #[code(Internal)]
    MappingError(String, #[source] crate::api::errors::DatabaseError),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for CustomerApiError {
    fn from(value: Report<StoreError>) -> Self {
        let mut err = value.current_context();

        loop {
            if let StoreError::TransactionStoreError(inner_report) = err {
                err = inner_report.current_context();
                continue;
            }
            return match err {
                StoreError::NegativeCustomerBalanceError(_) => {
                    Self::FailedPrecondition("negative customer balance".into())
                }
                _ => Self::StoreError(
                    "Error in customer service".to_string(),
                    Box::new(value.into_error()),
                ),
            };
        }
    }
}
