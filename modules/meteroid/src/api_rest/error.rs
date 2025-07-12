use serde::Serialize;

#[derive(Serialize)]
pub struct RestErrorResponse {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Serialize)]
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
    #[serde(rename = "INTERNAL_SERVER_ERROR")]
    InternalServerError,
}
