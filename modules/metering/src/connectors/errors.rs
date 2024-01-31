#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConnectorError {
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Failed to connect")]
    ResourceUnavailable,

    #[error("Failed to run connector initialization: {0}")]
    InitError(String),

    #[error("Failed to register meter")]
    RegisterError,

    #[error("Failed to query metering database")]
    QueryError,

    #[error("Invalid query : {0}")]
    InvalidQuery(String),
}
