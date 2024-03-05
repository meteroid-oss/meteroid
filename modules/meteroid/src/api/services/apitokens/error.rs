use deadpool_postgres::tokio_postgres;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum ApiTokenServiceError {
    #[error("Password hash error: {0}")]
    #[code(InvalidArgument)]
    PasswordHashError(String),

    #[error("Database error: {0}")]
    #[code(InvalidArgument)]
    DatabaseError(String, #[source] tokio_postgres::Error),
}
