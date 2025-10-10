use crate::api_rest::error::{ErrorCode, RestErrorResponse};
use axum::Json;
use axum::response::{IntoResponse, Response};
use error_stack::Report;
use hyper::StatusCode;
use meteroid_store::errors::StoreError;

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
    #[error("Event is missing metadata : {0}")]
    MissingMetadata(String),
    #[error("Invalid metadata")]
    InvalidMetadata,
    #[error("Error in payment provider")]
    ProviderError,
    #[error("Error in store")]
    StoreError,
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
            AdapterWebhookError::InvalidMetadata => StatusCode::BAD_REQUEST,
            AdapterWebhookError::MissingMetadata(_) => StatusCode::BAD_REQUEST,
            AdapterWebhookError::ObjectStoreUnreachable => StatusCode::INTERNAL_SERVER_ERROR,
            AdapterWebhookError::DatabaseError => StatusCode::INTERNAL_SERVER_ERROR,
            AdapterWebhookError::ProviderError => StatusCode::INTERNAL_SERVER_ERROR,
            AdapterWebhookError::StoreError => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let error_message = match status {
            StatusCode::INTERNAL_SERVER_ERROR => {
                "Internal server error. Please refer to logs or support.".to_string()
            }
            _ => format!("{self}"),
        };
        (status, error_message).into_response()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum WorkerError {
    #[error("Failed to update currency rates")]
    CurrencyRatesUpdateError,
}

#[derive(Debug, thiserror::Error)]
pub enum InvoicingRenderError {
    #[error("Failed to initialize invoice rendering service")]
    InitializationError,
    #[error("Invalid currency value: {0}")]
    InvalidCurrency(String),
    #[error("Failed to render invoice")]
    RenderError,
    #[error("Failed to generate pdf")]
    PdfError,
    #[error("Failed to store invoice document")]
    StorageError,
    #[error("Failed to load data to render invoice")]
    StoreError,
}

#[allow(unused)]
#[derive(Debug, thiserror::Error, Clone)]
pub enum RestApiError {
    #[error("Object store error")]
    ObjectStoreError,
    #[error("Failed to load image")]
    ImageLoadingError,
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Internal server error")]
    StoreError,
    #[error("Forbidden")]
    Forbidden,
    #[error("Invalid input")]
    InvalidInput,
    #[error("Resource not found")]
    NotFound,
    #[error("Conflict")]
    Conflict,
}

impl IntoResponse for RestApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            RestApiError::ObjectStoreError => {
                (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Unauthorized)
            }
            RestApiError::ImageLoadingError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InternalServerError,
            ),
            RestApiError::Unauthorized => (StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized),
            RestApiError::Forbidden => (StatusCode::FORBIDDEN, ErrorCode::Forbidden),
            RestApiError::InvalidInput => (StatusCode::BAD_REQUEST, ErrorCode::BadRequest),
            RestApiError::StoreError => {
                (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Unauthorized)
            }
            RestApiError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            RestApiError::Conflict => (StatusCode::CONFLICT, ErrorCode::Conflict),
        };

        let error_message = match status {
            StatusCode::INTERNAL_SERVER_ERROR => {
                "Internal server error. Please refer to logs or support.".to_string()
            }
            _ => format!("{self}"),
        };
        let error_body = Json(RestErrorResponse {
            code,
            message: error_message,
        });

        (status, error_body).into_response()
    }
}

impl From<Report<StoreError>> for RestApiError {
    fn from(err: Report<StoreError>) -> Self {
        match err.current_context() {
            StoreError::ValueNotFound(_) => RestApiError::NotFound,
            StoreError::DuplicateValue { .. } => RestApiError::Conflict,
            _ => RestApiError::StoreError,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ObjectStoreError {
    #[error("Failed to parse url")]
    InvalidUrl,
    #[error("Error saving object to object store")]
    SaveError,
    #[error("Error loading object from object store")]
    LoadError,
    #[error("Error signing url from object store")]
    SignedUrlError,
    #[error("Unsupported object store: {0}")]
    UnsupportedStore(String),
}
