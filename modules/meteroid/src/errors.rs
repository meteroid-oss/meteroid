use axum::response::{IntoResponse, Response};
use hyper::StatusCode;

#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum AdapterWebhookError {
    #[error("Endpoint is not registered")]
    UnknownEndpointId,
    #[error("Endpoint id is not valid")]
    InvalidEndpointId,
    #[error("Unknown provider : {0}")]
    UnknownProvider(String),
    #[error("Provider not supported : {0}")]
    ProviderNotSupported(String),
    #[error("Unauthorized request")]
    Unauthorized,
    #[error("Failed to decode body")]
    BodyDecodingFailed,
    #[error("Webhook event type not supported : {0}")]
    EventTypeNotSupported(String),
    #[error("Failed to verify webhook signature")]
    SignatureVerificationFailed,
    #[error("Failed to verify webhook signature")]
    SignatureNotFound,
    #[error("Failed to archive in object store")]
    ObjectStoreUnreachable,
    #[error("Database error")]
    DatabaseError,
    // DuplicateRequest,
}

impl IntoResponse for AdapterWebhookError {
    fn into_response(self) -> Response {
        let status = match self {
            AdapterWebhookError::UnknownEndpointId => StatusCode::NOT_FOUND,
            AdapterWebhookError::InvalidEndpointId => StatusCode::NOT_FOUND,
            AdapterWebhookError::UnknownProvider(_) => StatusCode::NOT_FOUND,
            AdapterWebhookError::ProviderNotSupported(_) => StatusCode::NOT_IMPLEMENTED,
            AdapterWebhookError::Unauthorized => StatusCode::UNAUTHORIZED,
            AdapterWebhookError::BodyDecodingFailed => StatusCode::BAD_REQUEST,
            AdapterWebhookError::EventTypeNotSupported(_) => StatusCode::BAD_REQUEST,
            AdapterWebhookError::SignatureVerificationFailed => StatusCode::FORBIDDEN,
            AdapterWebhookError::SignatureNotFound => StatusCode::BAD_REQUEST,
            AdapterWebhookError::ObjectStoreUnreachable => StatusCode::INTERNAL_SERVER_ERROR,
            AdapterWebhookError::DatabaseError => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let error_message = match status {
            StatusCode::INTERNAL_SERVER_ERROR => {
                "Internal server error. Please refer to logs or support.".to_string()
            }
            _ => format!("{}", self),
        };
        (status, error_message).into_response()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum WorkerError {
    #[error("Grpc Error")]
    GrpcError,
    #[error("Invalid input")]
    InvalidInput,
    #[error("Grpc Missing Field")]
    GrpcMissingField,
    #[error("Unknown pricing model")]
    UnknownPricingModel,
    #[error("Database error")]
    DatabaseError,
    #[error("Provider error")]
    ProviderError,
    #[error("Metering error")]
    MeteringError,
    #[error("Failed to update currency rates")]
    CurrencyRatesUpdateError,
}

#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum InvoicingAdapterError {
    #[error("Database error")]
    DatabaseError,
    #[error("Provider is not configured")]
    ProviderNotConfigured,
    #[error("Invalid Invoice Data")]
    InvalidData,
    #[error("Grpc error")]
    GrpcError,
    #[error("Stripe call error")]
    StripeError,
}
