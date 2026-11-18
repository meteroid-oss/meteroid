//! The extractors of the REST API. They read a request like axum's own, but a request they
//! refuse is answered with the same JSON error as every other failure, not with plain text.

use crate::error::{ErrorCode, RestErrorResponse};
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, OptionalFromRequest, Request};
use axum::response::{IntoResponse, Response};
use axum_valid::HasValidate;
use http::StatusCode;
use http::request::Parts;
use serde::de::DeserializeOwned;
use validator::Validate;

/// Why a request was refused before reaching its handler.
#[derive(Debug)]
pub struct Rejection {
    status: StatusCode,
    message: String,
}

impl Rejection {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        let code = if self.status.is_server_error() {
            ErrorCode::InternalServerError
        } else {
            ErrorCode::BadRequest
        };
        let body = RestErrorResponse {
            code,
            message: self.message,
        };
        (self.status, axum::Json(body)).into_response()
    }
}

macro_rules! from_axum_rejection {
    ($($rejection:ty),+) => {$(
        impl From<$rejection> for Rejection {
            fn from(rejection: $rejection) -> Self {
                Self {
                    status: rejection.status(),
                    message: rejection.body_text(),
                }
            }
        }
    )+};
}
from_axum_rejection!(JsonRejection, PathRejection, QueryRejection);

/// A JSON request body. Responses use `axum::Json`.
#[derive(Debug, Clone, Copy, Default, FromRequest)]
#[from_request(via(axum::Json), rejection(Rejection))]
pub struct ApiJson<T>(pub T);

/// `Option<ApiJson<T>>` is `None` when the request declares no content type.
impl<T, S> OptionalFromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Rejection;

    async fn from_request(req: Request, state: &S) -> Result<Option<Self>, Self::Rejection> {
        let body = <axum::Json<T> as OptionalFromRequest<S>>::from_request(req, state).await?;
        Ok(body.map(|axum::Json(value)| Self(value)))
    }
}

/// The path parameters of the route.
#[derive(Debug, Clone, Copy, Default, FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(Rejection))]
pub struct ApiPath<T>(pub T);

/// The query string, one value per key. Use [`crate::QueryParams`] when a key can repeat.
#[derive(Debug, Clone, Copy, Default, FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(Rejection))]
pub struct ApiQuery<T>(pub T);

macro_rules! has_validate {
    ($($extractor:ident),+) => {$(
        impl<T> HasValidate for $extractor<T> {
            type Validate = T;
            fn get_validate(&self) -> &T {
                &self.0
            }
        }
    )+};
}
has_validate!(ApiJson, ApiPath, ApiQuery);

/// Runs the `validator` rules of what the inner extractor read.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiValid<E>(pub E);

fn validated<E>(inner: E) -> Result<ApiValid<E>, Response>
where
    E: HasValidate,
    E::Validate: Validate,
{
    match inner.get_validate().validate() {
        Ok(()) => Ok(ApiValid(inner)),
        Err(errors) => Err(Rejection::bad_request(errors.to_string()).into_response()),
    }
}

impl<E, S> FromRequest<S> for ApiValid<E>
where
    E: FromRequest<S> + HasValidate,
    E::Validate: Validate,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let inner = E::from_request(req, state)
            .await
            .map_err(IntoResponse::into_response)?;
        validated(inner)
    }
}

impl<E, S> FromRequestParts<S> for ApiValid<E>
where
    E: FromRequestParts<S> + HasValidate,
    E::Validate: Validate,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let inner = E::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        validated(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use serde::Deserialize;

    #[derive(Debug, Deserialize, Validate)]
    struct Payment {
        #[validate(range(min = 1))]
        amount: i64,
    }

    fn post(content_type: Option<&str>, body: &str) -> Request {
        let mut request = Request::builder().method("POST").uri("/");
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        request.body(Body::from(body.to_owned())).unwrap()
    }

    async fn error_of(response: Response) -> (StatusCode, RestErrorResponse) {
        let status = response.status();
        assert_eq!(response.headers()["content-type"], "application/json");
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    async fn refused(request: Request) -> (StatusCode, RestErrorResponse) {
        let rejection = ApiValid::<ApiJson<Payment>>::from_request(request, &())
            .await
            .unwrap_err();
        error_of(rejection).await
    }

    #[tokio::test]
    async fn a_refused_body_keeps_its_status_and_answers_in_json() {
        let json = Some("application/json");
        for (request, status) in [
            (post(json, "{"), StatusCode::BAD_REQUEST),
            (post(json, "{}"), StatusCode::UNPROCESSABLE_ENTITY),
            (post(None, "{}"), StatusCode::UNSUPPORTED_MEDIA_TYPE),
            (post(json, r#"{"amount":0}"#), StatusCode::BAD_REQUEST),
        ] {
            let (refused_with, error) = refused(request).await;
            assert_eq!(refused_with, status);
            assert_eq!(error.code, ErrorCode::BadRequest);
            assert!(!error.message.is_empty());
        }
    }

    #[tokio::test]
    async fn a_valid_body_is_read() {
        let request = post(Some("application/json"), r#"{"amount":5}"#);
        let ApiValid(ApiJson(payment)) = ApiValid::<ApiJson<Payment>>::from_request(request, &())
            .await
            .unwrap();
        assert_eq!(payment.amount, 5);
    }

    #[tokio::test]
    async fn a_refused_query_answers_in_json() {
        #[derive(Debug, Deserialize)]
        struct Page {
            #[allow(dead_code)]
            page: u32,
        }
        let (mut parts, ()) = Request::builder()
            .uri("/?page=first")
            .body(())
            .unwrap()
            .into_parts();
        let rejection = ApiQuery::<Page>::from_request_parts(&mut parts, &())
            .await
            .unwrap_err();
        let (status, error) = error_of(rejection.into_response()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error.code, ErrorCode::BadRequest);
    }
}
