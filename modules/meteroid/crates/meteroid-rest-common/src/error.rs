use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct RestErrorResponse {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Serialize, Deserialize, ToSchema, PartialEq, Debug, Clone, Copy)]
pub enum ErrorCode {
    #[serde(rename = "BAD_REQUEST")]
    BadRequest,
    #[serde(rename = "NOT_FOUND")]
    NotFound,
    #[serde(rename = "CONFLICT")]
    Conflict,
    #[serde(rename = "FORBIDDEN")]
    Forbidden,
    #[serde(rename = "UNAUTHORIZED")]
    Unauthorized,
    #[serde(rename = "TOKEN_EXPIRED")]
    TokenExpired,
    #[serde(rename = "TOO_MANY_REQUESTS")]
    TooManyRequests,
    #[serde(rename = "INTERNAL_SERVER_ERROR")]
    InternalServerError,
    #[serde(rename = "RESOURCE_REQUIRED")]
    ResourceRequired,
    #[serde(rename = "NOT_SUBSCRIBED")]
    NotSubscribed,
    #[serde(rename = "RESOURCE_TYPE_ARCHIVED")]
    ResourceTypeArchived,
    #[serde(rename = "RESOURCE_LIMIT_REACHED")]
    ResourceLimitReached,
    #[serde(rename = "RESOURCE_HIERARCHY_UNSUPPORTED")]
    ResourceHierarchyUnsupported,
    #[serde(rename = "IDEMPOTENCY_KEY_REUSED")]
    IdempotencyKeyReused,
    #[serde(rename = "STALE_SET")]
    StaleSet,
    #[serde(rename = "RESOURCE_KEY_CLASH")]
    ResourceKeyClash,
}
