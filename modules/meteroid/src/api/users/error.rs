use deadpool_postgres::tokio_postgres;
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

    #[error("JWT error: {0}")]
    #[code(Internal)]
    JWTError(String, #[source] jsonwebtoken::errors::Error),

    #[error("Password hashing error: {0}")]
    #[code(Internal)]
    PasswordHashingError(String),

    #[error("Authentication error: {0}")]
    #[code(Unauthenticated)]
    AuthenticationError(String),

    #[error("User already exists error")]
    #[code(AlreadyExists)]
    UserAlreadyExistsError,

    #[error("Registration error: {0}")]
    #[code(PermissionDenied)]
    RegistrationClosed(String),

    #[error("Entity not found: {0}")]
    #[code(NotFound)]
    DatabaseEntityNotFoundError(String, #[source] tokio_postgres::Error),

    #[error("Database error: {0}")]
    #[code(Internal)]
    DatabaseError(String, #[source] tokio_postgres::Error),
}
