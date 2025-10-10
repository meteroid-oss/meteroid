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

    #[error("{0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for ConnectorApiError {
    fn from(value: Report<StoreError>) -> Self {
        let error_message = value.current_context().to_string();

        let err = Box::new(
            value
                .attach("Error in api connector service".to_string())
                .into_error(),
        );

        Self::StoreError(error_message, err)
    }
}
