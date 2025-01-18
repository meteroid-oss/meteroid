use diesel::result::Error;

use crate::compute::ComputeError;
use diesel_models::errors::DatabaseError;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Initialization Error: {0}")]
    InitializationError(String),
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
    #[error("Failed to cancel subscription")]
    CancellationError,
    #[error("Failed to insert subscription")]
    InsertError,
    #[error("Transaction error: {0:?}")]
    TransactionStoreError(error_stack::Report<StoreError>),
    #[error("Failed to compute invoice lines")]
    InvoiceComputationError,
    #[error("Failed to process price components: {0}")]
    InvalidPriceComponents(String),
    #[error("Failed to serialize/deserialize data: {0}")]
    SerdeError(String, #[source] serde_json::Error),
    #[error("Failed to encrypt/decrypt data")]
    CryptError(String),
    #[error("Login failure")]
    LoginError(String),
    #[error("Registration closed")]
    UserRegistrationClosed(String),
    #[error("Negative customer balance: {0:?}")]
    NegativeCustomerBalanceError(error_stack::Report<DatabaseError>),
    #[error("Metering Service error: {0}")]
    MeteringServiceError(String, #[source] ComputeError),
    #[error("Webhook Service error: {0}")]
    WebhookServiceError(String),
    #[error("Failed to send email")]
    MailServiceError,
}

// used in some o2o macros failing to compile, https://github.com/meteroid-oss/meteroid/actions/runs/10921372280/job/30313299862
pub(crate) type StoreErrorReport = error_stack::Report<StoreError>;

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
