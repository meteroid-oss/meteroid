use std::error::Error;

use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum WebhookApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    SvixError(String, #[source] Box<dyn Error>),
}

impl From<svix::error::Error> for WebhookApiError {
    fn from(value: svix::error::Error) -> Self {
        let err = Box::new(value);
        Self::SvixError("Error in webhook service".to_string(), err)
    }
}
