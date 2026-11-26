use axum::Json;
use axum::response::{IntoResponse, Response};
use error_stack::Report;
use http::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_app::rest::writes::store_error;
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::resources::ResourceRefusal;

/// A refusal answered with its own `code` (`RESOURCE_KEY_CLASH`, …), or any other REST error.
pub enum ApiError {
    Refused(ResourceRefusal, String),
    Rest(RestApiError),
}

impl From<Report<StoreError>> for ApiError {
    fn from(error: Report<StoreError>) -> Self {
        let refused =
            ResourceRefusal::of(&error).and_then(|refusal| match error.current_context() {
                StoreError::InvalidArgument(message) | StoreError::Conflict(message) => {
                    Some(Self::Refused(refusal, message.clone()))
                }
                _ => None,
            });
        refused.unwrap_or_else(|| Self::Rest(store_error(error)))
    }
}

impl From<RestApiError> for ApiError {
    fn from(error: RestApiError) -> Self {
        Self::Rest(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (refusal, message) = match self {
            Self::Rest(error) => return error.into_response(),
            Self::Refused(refusal, message) => (refusal, message),
        };
        let (status, code) = match refusal {
            ResourceRefusal::KeyClash => (StatusCode::CONFLICT, ErrorCode::ResourceKeyClash),
            ResourceRefusal::HierarchyUnsupported => (
                StatusCode::BAD_REQUEST,
                ErrorCode::ResourceHierarchyUnsupported,
            ),
            ResourceRefusal::LimitReached => {
                (StatusCode::BAD_REQUEST, ErrorCode::ResourceLimitReached)
            }
            ResourceRefusal::TypeArchived => {
                (StatusCode::BAD_REQUEST, ErrorCode::ResourceTypeArchived)
            }
        };
        (status, Json(RestErrorResponse { code, message })).into_response()
    }
}
