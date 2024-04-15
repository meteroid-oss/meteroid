use diesel_models::errors::DatabaseError;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Initialization Error")]
    InitializationError,
    #[error("DatabaseError: {0:?}")]
    DatabaseError(error_stack::Report<DatabaseError>),
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
}
