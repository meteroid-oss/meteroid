// use diesel_models::errors::DatabaseError;

use crate::StoreResult;
use diesel_models::errors::{DatabaseError, DatabaseErrorContainer};
use diesel_models::DbResult;
use error_stack::{Report, ResultExt};

pub type StorageResult<T> = error_stack::Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Initialization Error")]
    InitializationError,
    // // TODO: deprecate this error type to use a domain error instead
    #[error("DatabaseError: {0:?}")]
    DatabaseError(error_stack::Report<DatabaseError>),
    #[error("ValueNotFound: {0}")]
    ValueNotFound(String),
    #[error("DuplicateValue: {entity} already exists {key:?}")]
    DuplicateValue {
        entity: &'static str,
        key: Option<String>,
    },
    #[error("Timed out while trying to connect to the database")]
    DatabaseConnectionError,
}

impl From<DatabaseError> for StoreError {
    fn from(err: DatabaseError) -> Self {
        match err {
            DatabaseError::DatabaseConnectionError => StoreError::DatabaseConnectionError,
            DatabaseError::NotFound => {
                StoreError::ValueNotFound(String::from("db value not found"))
            }
            DatabaseError::UniqueViolation => StoreError::DuplicateValue {
                entity: "db entity",
                key: None,
            },
            _ => StoreError::DatabaseError(error_stack::report!(err)),
        }
    }
}
