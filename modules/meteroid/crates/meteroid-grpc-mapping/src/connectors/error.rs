use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum ConnectorApiError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    /// Invalid input with a user-facing message, shown as is.
    #[error("{0}")]
    #[code(InvalidArgument)]
    InvalidInput(String),

    /// User-facing: the connector's state doesn't allow the action.
    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("{0}")]
    #[code(NotFound)]
    NotFound(String),

    #[error("{0}")]
    #[code(AlreadyExists)]
    AlreadyExists(String),

    #[error("{0}")]
    #[code(FailedPrecondition)]
    Unavailable(String),

    #[error("{0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for ConnectorApiError {
    fn from(value: Report<StoreError>) -> Self {
        let error_message = value.current_context().to_string();

        match value.current_context() {
            StoreError::FailedPrecondition(msg) | StoreError::ConnectorNotActive(msg) => {
                return Self::FailedPrecondition(msg.clone());
            }
            StoreError::InvalidArgument(msg) => return Self::InvalidInput(msg.clone()),
            mismatch @ StoreError::ProviderMismatch { .. } => {
                return Self::InvalidInput(mismatch.to_string());
            }
            StoreError::ValueNotFound(_) => {
                return Self::NotFound("Connector not found".to_string());
            }
            StoreError::DuplicateValue { .. } => {
                return Self::AlreadyExists(
                    "A connection with this name already exists".to_string(),
                );
            }
            _ => {}
        }

        let err = Box::new(
            value
                .attach("Error in api connector service".to_string())
                .into_error(),
        );

        Self::StoreError(error_message, err)
    }
}
