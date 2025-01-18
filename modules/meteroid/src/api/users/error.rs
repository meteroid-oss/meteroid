use std::error::Error;

use error_stack::Report;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum UserApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Authentication error: {0}")]
    #[code(Unauthenticated)]
    AuthenticationError(String),

    #[error("A user with that email already exists.")]
    #[code(AlreadyExists)]
    UserAlreadyExistsError,

    #[error("Registration error: {0}")]
    #[code(PermissionDenied)]
    RegistrationClosed(String),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for UserApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = value.current_context();

        match err {
            StoreError::LoginError(str) => Self::AuthenticationError(str.clone()),
            StoreError::InvalidArgument(str) => Self::InvalidArgument(str.clone()),
            StoreError::DuplicateValue { entity: _, key: _ } => Self::UserAlreadyExistsError,
            StoreError::UserRegistrationClosed(value) => Self::RegistrationClosed(value.clone()),
            _e => Self::StoreError(
                "Error in user service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}
