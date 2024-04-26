use deadpool_postgres::tokio_postgres;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum UserApiError {
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Serialization error: {0}")]
    #[code(InvalidArgument)]
    SerializationError(String, #[source] serde_json::Error),

    #[error("Mapping error: {0}")]
    #[code(Internal)]
    MappingError(String, #[source] crate::api::errors::DatabaseError),

    #[error("Authentication error: {0}")]
    #[code(Unauthenticated)]
    AuthenticationError(String),

    #[error("User already exists error")]
    #[code(AlreadyExists)]
    UserAlreadyExistsError,

    #[error("Entity not found: {0}")]
    #[code(NotFound)]
    DatabaseEntityNotFoundError(String, #[source] tokio_postgres::Error),

    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] tokio_postgres::Error),

    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
}

impl Into<UserApiError> for error_stack::Report<meteroid_store::errors::StoreError> {
    fn into(self) -> UserApiError {
        let err = self.current_context();

        match err {
            meteroid_store::errors::StoreError::LoginError(str) => {
                UserApiError::AuthenticationError(str.clone())
            }
            meteroid_store::errors::StoreError::DuplicateValue { entity: _, key: _ } => {
                UserApiError::UserAlreadyExistsError
            }
            _e => UserApiError::StoreError(
                "Error in user service".to_string(),
                Box::new(self.into_error()),
            ),
        }
    }
}
