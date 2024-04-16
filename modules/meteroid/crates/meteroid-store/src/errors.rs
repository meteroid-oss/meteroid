use diesel::result::Error;
use diesel_models::errors::DatabaseError;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Initialization Error")]
    InitializationError,
    #[error("DatabaseError: {0:?}")]
    DatabaseError(error_stack::Report<DatabaseError>),
    #[error("ValueNotFound: {0}")]
    ValueNotFound(String),
    #[error("DuplicateValue: {entity} already exists {key:?}")]
    DuplicateValue {
        entity: &'static str,
        key: Option<String>,
    },
    #[error("Invalid Argument: {0}")]
    InvalidArgument(String),
    #[error("Timed out while trying to connect to the database")]
    DatabaseConnectionError,
    #[error("Invalid decimal value")]
    InvalidDecimal,
    #[error("Failed to process price components: {0}")]
    InvalidPriceComponents(String),
    #[error("Failed to serialize/deserialize data: {0}")]
    SerdeError(String, #[source] serde_json::Error),
    #[error("Transaction error: {0:?}")]
    TransactionStoreError(error_stack::Report<StoreError>),
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

impl From<diesel::result::Error> for StoreError {
    fn from(value: Error) -> Self {
        DatabaseError::from(&value).into()
    }
}
