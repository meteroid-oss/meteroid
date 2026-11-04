use diesel::result::Error;
use diesel_models::errors::DatabaseError;
use error_stack::IntoReport;

/// Attached to a refusal caused by a rate limit, so an API can answer "too many requests".
#[derive(Debug, Clone)]
pub struct RateLimited(pub String);

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
    /// The request conflicts with one already recorded under the same identity.
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Invalid Argument: {0}")]
    InvalidArgument(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
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
    #[error("Error in metering client")]
    MeteringServiceError,
    #[error("The metering service does not support usage filters yet; retry shortly")]
    MeteringNotReady,
    #[error("Usage could not be read in time; retry shortly")]
    UsageUnavailable,
    #[error("Webhook Service error: {0}")]
    WebhookServiceError(String),
    #[error("Failed to send email")]
    MailServiceError,
    #[error("Error in object store service")]
    ObjectStoreError,
    #[error("Error received from payment provider")]
    PaymentProviderError,
    #[error("Payment failed or pending: {0}")]
    PaymentError(String),
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
    /// A third-party API answered with an error or could not be reached; names the service.
    #[error("Error received from {0}")]
    ExternalServiceError(String),
    /// The current state doesn't allow the action. The message is fit for the end user.
    #[error("{0}")]
    FailedPrecondition(String),
    /// The connector is a draft, disabled or deleted, so it can't start a payment.
    #[error("{0}")]
    ConnectorNotActive(String),
    /// Credentials were sent to a connector that belongs to another provider: a client bug, or a
    /// page left open while the connection was replaced. Its own variant so the caller can tell
    /// it apart from a typo in a key.
    #[error("These credentials are for {sent:?}, but this connection is a {stored:?} one")]
    ProviderMismatch { sent: String, stored: String },
}

impl StoreError {
    pub fn is_payment_error(&self) -> bool {
        matches!(
            self,
            StoreError::PaymentError(_) | StoreError::PaymentProviderError
        )
    }
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
