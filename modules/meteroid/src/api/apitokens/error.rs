use std::error::Error;

use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum ApiTokenApiError {
    #[error("Password hash error: {0}")]
    #[code(Internal)]
    PasswordHashError(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}
