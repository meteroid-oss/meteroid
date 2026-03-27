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
    #[serde(rename = "TOO_MANY_REQUESTS")]
    TooManyRequests,
    #[serde(rename = "INTERNAL_SERVER_ERROR")]
    InternalServerError,
}
