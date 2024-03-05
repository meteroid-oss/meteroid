use deadpool_postgres::tokio_postgres;
use error_stack::Report;
use thiserror::Error;

use crate::repo::errors::RepoError;
use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum TenantServiceError {
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Downstream service error: {0}")]
    #[code(InvalidArgument)]
    DownstreamServiceError(String, #[source] Report<RepoError>),

    #[error("Database error: {0}")]
    #[code(InvalidArgument)]
    DatabaseError(String, #[source] tokio_postgres::Error),
}
