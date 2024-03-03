use std::error::Error;

use deadpool_postgres::tokio_postgres;
use thiserror::Error;

use common_grpc_source_details_macros::SourceDetailsError;
use common_grpc_source_details_macros_impl::SourceDetailsError;

#[derive(Debug, Error, SourceDetailsError)]
pub enum CustomerServiceError {
    #[error("Unknown error occurred: {0}")]
    UnknownError(String),

    #[error("Database error: {0}")]
    DatabaseError(String, #[source] tokio_postgres::Error),
}
