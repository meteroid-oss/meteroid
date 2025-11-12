use diesel::result::Error;
use diesel_models::errors::DatabaseError;
use error_stack::IntoReport;

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
    #[error("Failed to compute invoice lines")]
    InvoiceComputationError,
    #[error("Failed to bill subscription")]
    BillingError,
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
    #[error("Error in metering client")]
    MeteringServiceError,
    #[error("Webhook Service error: {0}")]
    WebhookServiceError(String),
    #[error("Failed to send email")]
    MailServiceError,
    #[error("Error in object store service")]
    ObjectStoreError,
    #[error("Error received from payment provider")]
    PaymentProviderError,
    #[error("OAuth failure: {0}")]
    OauthError(String),
    #[error("Checkout could not complete")]
    CheckoutError,
    #[error("Provider is not connected")]
    ProviderNotConnected,
    #[error("Invalid date value")]
    InvalidDate,
    #[error("Failed to compute taxes")]
    TaxError,
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
            DatabaseError::ValidationError(msg) => StoreError::InvalidArgument(msg),
            _ => StoreError::DatabaseError(err.into_report()),
        }
    }
}

impl From<Error> for StoreError {
    fn from(value: Error) -> Self {
        DatabaseError::from(&value).into()
    }
}

// Container type used for transactions
#[derive(Debug)]
pub struct StoreErrorContainer {
    pub error: error_stack::Report<StoreError>,
}

impl From<error_stack::Report<StoreError>> for StoreErrorContainer {
    fn from(error: error_stack::Report<StoreError>) -> Self {
        Self {
            error: error.attach("Transaction failed"),
        }
    }
}

impl From<Error> for StoreErrorContainer {
    fn from(error: Error) -> Self {
        let store_error = StoreError::from(DatabaseError::from(&error));
        Self {
            error: error_stack::Report::from(error).change_context(store_error),
        }
    }
}

impl From<StoreErrorContainer> for error_stack::Report<StoreError> {
    fn from(container: StoreErrorContainer) -> Self {
        container.error
    }
}
