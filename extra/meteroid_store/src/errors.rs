// use diesel_models::errors::DatabaseError;

use diesel_models::errors::DatabaseError;
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
    // #[error("KV error")]
    // KVError,
    // #[error("Serialization failure")]
    // SerializationFailed,
    // #[error("MockDb error")]
    // MockDbError,
    // #[error("Kafka error")]
    // KafkaError,
    // #[error("Customer with this id is Redacted")]
    // CustomerRedacted,
    // #[error("Deserialization failure")]
    // DeserializationFailed,
    // #[error("Error while encrypting data")]
    // EncryptionError,
    // #[error("Error while decrypting data from database")]
    // DecryptionError,
    // // TODO: deprecate this error type to use a domain error instead
    // #[error("RedisError: {0:?}")]
    // RedisError(String),
}

// TODO switch impl like hyperswitch ?
pub fn db_error_to_store(err: Report<DatabaseError>) -> Report<StoreError> {
    match err.current_context() {
        DatabaseError::DatabaseConnectionError => {
            Report::from(err).change_context(StoreError::DatabaseConnectionError)
        }
        // TODO: Update this error type to encompass & propagate the missing type
        DatabaseError::NotFound => Report::from(err).change_context(StoreError::ValueNotFound(
            String::from("db value not found"),
        )),
        // TODO: Update this error type to encompass & propagate the duplicate type
        DatabaseError::UniqueViolation => {
            Report::from(err).change_context(StoreError::DuplicateValue {
                entity: "db entity",
                key: None,
            })
        }
        err => Report::from(StoreError::DatabaseError(error_stack::report!(*err))),
    }
}
