//! What every v2 response carries, whoever answered it (ADR-0013 §2, §3): the resolved
//! `Meteroid-Version`, a `Request-Id`, and errors as problems (a `401` with `WWW-Authenticate:
//! Bearer`). Mounted outside authentication, rate limiting and the body limit, so their refusals
//! (answered in v1's `{code, message}`), the unknown routes and the panics read as problems too.
//! A protocol endpoint forwarded as it is (OAuth's) marks its responses [`Verbatim`]: its errors
//! keep their protocol's shape. The client API (`/api/client/`) follows v2's conventions, and is
//! enveloped alike.
use crate::client::CLIENT_API_PREFIX;
use crate::v2::problem::{PROBLEM_CONTENT_TYPE, Problem, ProblemCode};
use crate::v2::version::{ApiVersion, VERSION_HEADER, default_version, resolve};
use axum::body::Body;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::{HeaderValue, Request, StatusCode};
use meteroid_rest_common::error::RestErrorResponse;

pub const PREFIX: &str = "/api/v2";
pub const REQUEST_ID_HEADER: &str = "Request-Id";
pub const CALLER_REQUEST_ID_HEADER: &str = "X-Request-Id";
const ERROR_BODY_LIMIT: usize = 64 * 1024;

/// The id of the request: the caller's `X-Request-Id` when it is a short visible string, else a
/// new one. In the request extensions, for handlers that log it.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// A response extension a handler sets when its error body is a protocol's own (OAuth's RFC 6749
/// errors, ADR-0013 §13, §20): the envelope keeps it as answered instead of rewriting it as a
/// problem, and still echoes the version and the request id.
#[derive(Debug, Clone, Copy)]
pub struct Verbatim;

/// `response` kept as answered by the envelope, its errors included.
pub fn verbatim(mut response: Response) -> Response {
    response.extensions_mut().insert(Verbatim);
    response
}

pub fn is_v2(path: &str) -> bool {
    path == PREFIX || path.starts_with("/api/v2/")
}

/// The paths answered in v2's conventions: v2's and the client API's.
pub fn is_enveloped(path: &str) -> bool {
    is_v2(path) || path.starts_with(CLIENT_API_PREFIX)
}

fn request_id_of(req: &Request<Body>) -> String {
    req.headers()
        .get(CALLER_REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_graphic()))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("req_{}", uuid::Uuid::now_v7().simple()))
}

pub async fn v2_envelope(mut req: Request<Body>, next: Next) -> Response {
    if !is_enveloped(req.uri().path()) {
        return next.run(req).await;
    }
    let request_id = request_id_of(&req);
    let asked = req
        .headers()
        .get(VERSION_HEADER)
        .map(|value| value.to_str().unwrap_or("?").to_owned());
    let (version, response) = match resolve(asked.as_deref()) {
        Ok(version) => {
            req.extensions_mut().insert(version);
            req.extensions_mut().insert(RequestId(request_id.clone()));
            (version, next.run(req).await)
        }
        Err(problem) => (default_version(), problem.into_response()),
    };
    let failed = response.status().is_client_error() || response.status().is_server_error();
    let mut response = if failed && response.extensions().get::<Verbatim>().is_none() {
        crate::auth::challenged(as_problem(response, &request_id).await)
    } else {
        response
    };
    echo(&mut response, version, &request_id);
    response
}

fn echo(response: &mut Response, version: ApiVersion, request_id: &str) {
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&version.as_header()) {
        headers.insert(VERSION_HEADER, value);
    }
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.insert(REQUEST_ID_HEADER, value);
    }
}

/// The error response as a problem carrying the request id: a problem keeps its fields, a v1
/// `{code, message}` gives its message, anything else its status.
pub(crate) async fn as_problem(response: Response, request_id: &str) -> Response {
    let (mut parts, body) = response.into_parts();
    let status = parts.status;
    let is_problem = parts
        .headers
        .get(http::header::CONTENT_TYPE)
        .is_some_and(|value| {
            value
                .as_bytes()
                .starts_with(PROBLEM_CONTENT_TYPE.as_bytes())
        });
    let bytes = axum::body::to_bytes(body, ERROR_BODY_LIMIT)
        .await
        .unwrap_or_default();
    let mut problem = if is_problem {
        serde_json::from_slice::<Problem>(&bytes)
            .unwrap_or_else(|_| Problem::new(status, ProblemCode::of_status(status), ""))
    } else {
        let detail = serde_json::from_slice::<RestErrorResponse>(&bytes)
            .map(|error| error.message)
            .unwrap_or_else(|_| {
                std::str::from_utf8(&bytes)
                    .ok()
                    .filter(|text| text.len() <= 512)
                    .unwrap_or_default()
                    .to_owned()
            });
        let detail = if status == StatusCode::INTERNAL_SERVER_ERROR {
            "Internal error. Quote the request_id to support.".to_owned()
        } else {
            detail
        };
        Problem::new(status, ProblemCode::of_status(status), detail)
    };
    problem.request_id = Some(request_id.to_owned());
    parts.headers.remove(http::header::CONTENT_LENGTH);
    parts.headers.insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
    );
    let body = serde_json::to_vec(&problem).unwrap_or_default();
    Response::from_parts(parts, Body::from(body))
}
