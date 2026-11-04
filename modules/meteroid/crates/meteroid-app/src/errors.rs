use axum::Json;
use axum::response::{IntoResponse, Response};
use error_stack::Report;
use hyper::StatusCode;
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_store::errors::StoreError;

#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum AdapterWebhookError {
    #[error("Endpoint is not registered")]
    UnknownEndpointId,
    #[error("Connector was deleted")]
    ConnectorDeleted,
    #[error("Provider not supported : {0}")]
    ProviderNotSupported(String),
    #[error("Failed to decode body")]
    BodyDecodingFailed,
    #[error("Webhook payload too large")]
    PayloadTooLarge,
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
}

impl AdapterWebhookError {
    /// Only transient failures (DB, provider fetch) are retried via pgmq; everything else is acked
    /// so a bad event can't block the queue.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::ProviderError
                | Self::StoreError
                | Self::DatabaseError
                | Self::ObjectStoreUnreachable
        )
    }
}

impl IntoResponse for AdapterWebhookError {
    fn into_response(self) -> Response {
        let status = match self {
            AdapterWebhookError::UnknownEndpointId => StatusCode::NOT_FOUND,
            // Tells providers that retry on errors (Stripe) this endpoint is gone for good.
            AdapterWebhookError::ConnectorDeleted => StatusCode::GONE,
            AdapterWebhookError::ProviderNotSupported(_) => StatusCode::NOT_IMPLEMENTED,
            AdapterWebhookError::BodyDecodingFailed => StatusCode::BAD_REQUEST,
            AdapterWebhookError::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
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
    #[error("Failed to cleanup checkout sessions")]
    CheckoutSessionCleanupError,
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
    #[error("The invoice does not satisfy the e-invoicing standard it is issued under")]
    EInvoicingError,
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
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Resource not found")]
    NotFound,
    #[error("Conflict")]
    Conflict,
    #[error("{0}")]
    FailedPrecondition(String),
    #[error("Service not configured: {0}")]
    NotConfigured(String),
    #[error("Service unavailable: {0}")]
    Unavailable(String),
    #[error("Feature not implemented: {0}")]
    NotImplemented(String),
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("External service error: {0}")]
    ExternalServiceError(String),
    #[error("Rate limit exceeded. {0}")]
    RateLimitExceeded(String),
}

impl IntoResponse for RestApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            RestApiError::ObjectStoreError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InternalServerError,
            ),
            RestApiError::ImageLoadingError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InternalServerError,
            ),
            RestApiError::Unauthorized => (StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized),
            RestApiError::Forbidden => (StatusCode::FORBIDDEN, ErrorCode::Forbidden),
            RestApiError::InvalidInput(_) => (StatusCode::BAD_REQUEST, ErrorCode::BadRequest),
            RestApiError::StoreError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InternalServerError,
            ),
            RestApiError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            RestApiError::Conflict => (StatusCode::CONFLICT, ErrorCode::Conflict),
            RestApiError::FailedPrecondition(_) => (StatusCode::CONFLICT, ErrorCode::Conflict),
            RestApiError::NotConfigured(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::InternalServerError,
            ),
            RestApiError::Unavailable(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::InternalServerError,
            ),
            RestApiError::NotImplemented(_) => {
                (StatusCode::NOT_IMPLEMENTED, ErrorCode::InternalServerError)
            }
            RestApiError::DatabaseError(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InternalServerError,
            ),
            RestApiError::ExternalServiceError(_) => {
                (StatusCode::BAD_GATEWAY, ErrorCode::InternalServerError)
            }
            RestApiError::RateLimitExceeded(_) => {
                (StatusCode::TOO_MANY_REQUESTS, ErrorCode::TooManyRequests)
            }
        };

        // 4xx are logged with request context at the call site / middleware
        if status.is_server_error() {
            log::error!("Responding with HTTP {status}: {self:?}");
        } else {
            log::debug!("Responding with HTTP {status}: {self:?}");
        }

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
            StoreError::InvalidArgument(msg) => RestApiError::InvalidInput(msg.clone()),
            StoreError::UsageUnavailable | StoreError::MeteringNotReady => {
                RestApiError::Unavailable(err.current_context().to_string())
            }
            _ => {
                // the response body only exposes an opaque 500, so keep the
                // full error chain (attachments + source location) in the logs
                log::error!("Store error while handling REST request: {err:?}");
                RestApiError::StoreError
            }
        }
    }
}

impl From<tonic::Status> for RestApiError {
    fn from(status: tonic::Status) -> Self {
        match status.code() {
            tonic::Code::NotFound => RestApiError::NotFound,
            tonic::Code::AlreadyExists => RestApiError::Conflict,
            tonic::Code::InvalidArgument => {
                RestApiError::InvalidInput(status.message().to_string())
            }
            tonic::Code::Unauthenticated => RestApiError::Unauthorized,
            tonic::Code::PermissionDenied => RestApiError::Forbidden,
            code => {
                log::error!(
                    "gRPC error while handling REST request: {code:?} {}",
                    status.message()
                );
                RestApiError::StoreError
            }
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
