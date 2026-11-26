//! `Idempotency-Key` on every v2 `POST` (ADR-0013 §3), on v1's `api_idempotency_key` claims: the
//! first call with a key runs and its response is kept; a retry with the same request replays
//! it (`Idempotent-Replayed: true`), with its status, its content type and its body as they were;
//! the same key with another request is refused (422), and a retry while the first still runs is
//! refused (409). A call that fails before it writes releases its key; once a handler marks its
//! write made ([`Committed`], [`committed_write`]) the key keeps the response, whatever its
//! status. The call runs in a task of its own: a client gone mid-request
//! never leaves its key claimed without an answer. A key still in flight after a crash answers
//! 409 until the retention job frees it (24 hours).
use crate::AppState;
use crate::v2::problem::{Problem, ProblemCode};
use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::{HeaderValue, Method, Request, StatusCode, response::Parts};
use meteroid_store::domain::credit::ApiIdempotency;
use meteroid_store::repositories::{ApiIdempotentResponsesInterface, ApiResponseClaim};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";
pub const REPLAYED_HEADER: &str = "Idempotent-Replayed";
/// Every v2 write shares one key space per tenant: a key names one request.
const OPERATION: &str = "v2";
const BODY_LIMIT: usize = 1024 * 1024;
/// A response larger than this is kept as a marker: its retry is told to read the resource.
const STORED_RESPONSE_LIMIT: usize = 512 * 1024;
/// A marker kept in a response's place is JSON, whatever the response was.
const MARKER_CONTENT_TYPE: &str = "application/json";

/// What a marker says of the first response: it was not kept.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdempotentReplay {
    ResponseNotKept,
}

/// What a retry with an `Idempotency-Key` replays when the first response was not kept (larger
/// than 512 KB, or holding a secret): its write was made and never runs again; read the resource
/// for its result, by `id` when the first response named one.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct IdempotentReplayMarker {
    pub idempotent_replay: IdempotentReplay,
    /// The first response's status, which the replay answers with too.
    pub status: u16,
    /// The `id` of the first response, when it was a JSON object with one: the resource the
    /// write created or changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub detail: String,
}

/// A response extension a handler sets once the store committed its write: the response is kept
/// for the key's retries whatever its status, and the key is never released (a retry must not
/// write again).
#[derive(Debug, Clone, Copy)]
pub struct Committed;

/// `response` marked as answering a committed write.
pub fn committed(mut response: Response) -> Response {
    response.extensions_mut().insert(Committed);
    response
}

/// The answer to a write the store committed (ADR-0013 §3: a committed write never answers an
/// error for what it reads back): `read`, what was written as v2 reads it back, or when that read
/// failed `known()`, what the write itself returned (its id and what it was sent). Marked
/// [`Committed`] either way: its key keeps it, a retry replays it and never writes again. A
/// handler calls it once its last write returned; a refusal before that answers as an error and
/// releases the key.
pub fn committed_write<T: Serialize, K: Serialize>(
    status: StatusCode,
    read: Result<T, Problem>,
    known: impl FnOnce() -> K,
) -> Response {
    let response = match read {
        Ok(body) => (status, Json(body)).into_response(),
        Err(problem) => {
            log::warn!(
                "A v2 write was made but is not read back, answered with what it wrote: {problem:?}"
            );
            (status, Json(known())).into_response()
        }
    };
    committed(response)
}

/// A response extension a handler sets when its body holds a secret (a webhook signing secret):
/// the key keeps a marker in its place, never the secret, and its retry is told to read the
/// resource, the secret having a read of its own.
#[derive(Debug, Clone, Copy)]
pub struct HoldsSecret;

/// `response` marked as holding a secret, which the key never keeps.
pub fn holds_secret(mut response: Response) -> Response {
    response.extensions_mut().insert(HoldsSecret);
    response
}

/// A JSON value with its object keys sorted, so two bodies alike but for key order fingerprint
/// alike.
fn canonical(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(String, serde_json::Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonical(value)))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonical).collect())
        }
        other => other,
    }
}

fn fingerprint(method: &Method, path: &str, body: &[u8]) -> Result<[u8; 32], Problem> {
    let body = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice::<serde_json::Value>(body)
            .map(canonical)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(body).into()))
    };
    meteroid_app::rest::writes::fingerprint(&(method.as_str(), path, body))
        .map_err(|_| Problem::internal())
}

/// A POST that only reads: the entitlement check. It takes no key, so no answer is kept.
pub fn reads_only(path: &str) -> bool {
    path.ends_with("/entitlements/check")
}

pub async fn idempotency(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if req.method() != Method::POST || reads_only(req.uri().path()) {
        return next.run(req).await;
    }
    let Some(key) = req
        .headers()
        .get(IDEMPOTENCY_KEY_HEADER)
        .map(|value| value.to_str().map(str::to_owned))
    else {
        return next.run(req).await;
    };
    let Ok(key) = key else {
        return Problem::field(
            IDEMPOTENCY_KEY_HEADER,
            "invalid",
            "An Idempotency-Key is visible ASCII",
        )
        .into_response();
    };
    let Some(auth) = req.extensions().get::<AuthorizedAsTenant>().cloned() else {
        return next.run(req).await;
    };

    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Problem::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                ProblemCode::PayloadTooLarge,
                "The request body is larger than 1 MB",
            )
            .into_response();
        }
    };
    let path = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| parts.uri.path());
    let identity = match fingerprint(&parts.method, path, &bytes).and_then(|fp| {
        ApiIdempotency::new(OPERATION, &key, fp).map_err(|error| {
            Problem::field(
                IDEMPOTENCY_KEY_HEADER,
                "invalid",
                crate::v2::problem::store_message(error.current_context()),
            )
        })
    }) {
        Ok(identity) => identity,
        Err(problem) => return problem.into_response(),
    };

    match state
        .store
        .claim_api_response(auth.tenant_id, &identity)
        .await
    {
        Err(error) => Problem::from(error).into_response(),
        Ok(ApiResponseClaim::Mismatch) => Problem::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::IdempotencyKeyReused,
            "This Idempotency-Key was used with another request: use a new key for a new request",
        )
        .into_response(),
        Ok(ApiResponseClaim::InFlight) => Problem::conflict(
            ProblemCode::IdempotencyRequestInProgress,
            "The first request with this Idempotency-Key is still running: retry later",
        )
        .into_response(),
        Ok(ApiResponseClaim::Replay {
            status,
            content_type,
            body,
        }) => replayed(status, content_type.as_deref(), body),
        Ok(ApiResponseClaim::Fresh) => {
            // A task of its own: a client gone mid-request cancels neither the write nor `settle`.
            let task = {
                let (state, auth, identity) = (state.clone(), auth.clone(), identity.clone());
                tokio::spawn(async move {
                    let response = next
                        .run(Request::from_parts(parts, Body::from(bytes)))
                        .await;
                    settle(&state, &auth, &identity, response).await
                })
            };
            match task.await {
                Ok(response) => response,
                Err(error) => {
                    // Whether it wrote is unknown: the key keeps a failure, never released.
                    log::error!("A v2 request with an Idempotency-Key failed: {error}");
                    let response = Problem::internal().into_response();
                    let (parts, body) = response.into_parts();
                    let bytes = axum::body::to_bytes(body, STORED_RESPONSE_LIMIT)
                        .await
                        .unwrap_or_default();
                    if let Err(error) = state
                        .store
                        .store_api_response(
                            auth.tenant_id,
                            &identity,
                            parts.status.as_u16(),
                            content_type_of(&parts),
                            &bytes,
                        )
                        .await
                    {
                        log::error!("Failed to keep a response for its Idempotency-Key: {error:?}");
                    }
                    Response::from_parts(parts, Body::from(bytes))
                }
            }
        }
    }
}

/// The response kept for a key, replayed as it was first answered: its status, its content type
/// (a problem stays a problem, the envelope reads its fields) and its body.
fn replayed(status: u16, content_type: Option<&str>, body: Vec<u8>) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    let mut response = (status, body).into_response();
    // Kept before the content type was (or never sent): every v2 answer but a problem is JSON.
    let content_type = content_type
        .and_then(|value| HeaderValue::from_str(value).ok())
        .unwrap_or_else(|| HeaderValue::from_static("application/json"));
    response
        .headers_mut()
        .insert(http::header::CONTENT_TYPE, content_type);
    response
        .headers_mut()
        .insert(REPLAYED_HEADER, HeaderValue::from_static("true"));
    response
}

fn content_type_of(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
}

/// Whether the key keeps `response` for its retries: a success, or any answer to a committed
/// write. Any other is a failure that wrote nothing, and releases the key.
fn keeps(response: &Response) -> bool {
    response.extensions().get::<Committed>().is_some() || response.status().is_success()
}

/// The `id` of a JSON object response.
fn id_of(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

fn marker(status: StatusCode, body: &[u8], detail: &str) -> Vec<u8> {
    serde_json::to_vec(&IdempotentReplayMarker {
        idempotent_replay: IdempotentReplay::ResponseNotKept,
        status: status.as_u16(),
        id: id_of(body),
        detail: detail.to_owned(),
    })
    .unwrap_or_default()
}

/// What a retry of a response too large to keep replays: the write was made, read its result.
fn too_large_marker(status: StatusCode, body: &[u8]) -> Vec<u8> {
    marker(
        status,
        body,
        "The first response with this Idempotency-Key was too large to keep: its write was made, \
         read the resource for its result",
    )
}

/// What a retry of a response holding a secret replays: the write was made, read its result.
fn secret_marker(status: StatusCode, body: &[u8]) -> Vec<u8> {
    marker(
        status,
        body,
        "The first response with this Idempotency-Key held a secret, which is never kept: its \
         write was made, read the resource and its secret for its result",
    )
}

/// Keeps a successful response, or any answering a committed write, for the key's retries;
/// releases the key of a failed one that wrote nothing.
async fn settle(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    identity: &ApiIdempotency,
    response: Response,
) -> Response {
    let holds_secret = response.extensions().get::<HoldsSecret>().is_some();
    if !keeps(&response) {
        if let Err(error) = state
            .store
            .release_api_response(auth.tenant_id, identity)
            .await
        {
            log::error!("Failed to release an idempotency key: {error:?}");
        }
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, BODY_LIMIT * 16).await {
        Ok(bytes) => bytes,
        Err(error) => {
            // Unreadable: the write is done all the same, its retry must not run it again.
            log::error!("A v2 response could not be read for its Idempotency-Key: {error}");
            let marker = too_large_marker(parts.status, &[]);
            if let Err(error) = state
                .store
                .store_api_response(
                    auth.tenant_id,
                    identity,
                    parts.status.as_u16(),
                    Some(MARKER_CONTENT_TYPE),
                    &marker,
                )
                .await
            {
                log::error!("Failed to keep a response for its Idempotency-Key: {error:?}");
            }
            return Problem::internal().into_response();
        }
    };
    let (content_type, kept) = if holds_secret {
        (
            Some(MARKER_CONTENT_TYPE),
            secret_marker(parts.status, &bytes),
        )
    } else if bytes.len() > STORED_RESPONSE_LIMIT {
        log::warn!(
            "A v2 response of {} bytes was kept as a marker for its Idempotency-Key",
            bytes.len()
        );
        (
            Some(MARKER_CONTENT_TYPE),
            too_large_marker(parts.status, &bytes),
        )
    } else {
        (content_type_of(&parts), bytes.to_vec())
    };
    if let Err(error) = state
        .store
        .store_api_response(
            auth.tenant_id,
            identity,
            parts.status.as_u16(),
            content_type,
            &kept,
        )
        .await
    {
        log::error!("Failed to keep a response for its Idempotency-Key: {error:?}");
    }
    Response::from_parts(parts, Body::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::problem::PROBLEM_CONTENT_TYPE;

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// What `settle` keeps of `response`: its parts and its body.
    fn kept(response: Response) -> (Parts, Vec<u8>) {
        assert!(keeps(&response), "the key keeps it");
        let (parts, body) = response.into_parts();
        let bytes = block_on(axum::body::to_bytes(body, BODY_LIMIT)).unwrap();
        (parts, bytes.to_vec())
    }

    fn body_of(response: Response) -> serde_json::Value {
        let bytes = block_on(axum::body::to_bytes(response.into_body(), BODY_LIMIT)).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn a_write_not_read_back_answers_what_it_wrote_and_its_retry_replays_it() {
        let answer = committed_write(
            StatusCode::CREATED,
            Err::<serde_json::Value, _>(Problem::internal()),
            || serde_json::json!({"id": "prod_1", "name": "Seats"}),
        );
        assert_eq!(answer.status(), StatusCode::CREATED, "never an error");
        let (parts, body) = kept(answer);
        assert_eq!(content_type_of(&parts), Some("application/json"));

        let replay = replayed(parts.status.as_u16(), content_type_of(&parts), body.clone());
        assert_eq!(replay.status(), StatusCode::CREATED);
        assert_eq!(replay.headers().get(REPLAYED_HEADER).unwrap(), "true");
        assert_eq!(
            replay.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert_eq!(
            body_of(replay),
            serde_json::json!({"id": "prod_1", "name": "Seats"})
        );
    }

    #[test]
    fn a_write_read_back_answers_as_read() {
        let answer = committed_write(
            StatusCode::OK,
            Ok(serde_json::json!({"id": "prod_1", "family": "core"})),
            || serde_json::json!({"id": "prod_1"}),
        );
        assert!(answer.extensions().get::<Committed>().is_some());
        let (_, body) = kept(answer);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"id": "prod_1", "family": "core"})
        );
    }

    #[test]
    fn a_failure_that_wrote_nothing_releases_its_key() {
        assert!(!keeps(&Problem::internal().into_response()));
        assert!(!keeps(
            &Problem::conflict(ProblemCode::Conflict, "taken").into_response()
        ));
    }

    #[test]
    fn a_problem_kept_replays_as_the_problem_it_was() {
        let first = committed(
            Problem::conflict(
                ProblemCode::Conflict,
                "Invoice inv_1 was created as a draft but not issued",
            )
            .into_response(),
        );
        let (parts, body) = kept(first);
        assert_eq!(content_type_of(&parts), Some(PROBLEM_CONTENT_TYPE));

        let replay = replayed(parts.status.as_u16(), content_type_of(&parts), body);
        assert_eq!(
            replay.headers().get(http::header::CONTENT_TYPE).unwrap(),
            PROBLEM_CONTENT_TYPE
        );
        // As the envelope answers it: the problem's own fields, the retry's request id.
        let answered = block_on(crate::v2::envelope::as_problem(replay, "req_retry"));
        assert_eq!(answered.status(), StatusCode::CONFLICT);
        let problem = body_of(answered);
        assert_eq!(problem["code"], "conflict");
        assert_eq!(
            problem["detail"],
            "Invoice inv_1 was created as a draft but not issued"
        );
        assert_eq!(problem["request_id"], "req_retry");
    }

    #[test]
    fn a_marker_names_what_the_write_made() {
        let marker: serde_json::Value = serde_json::from_slice(&secret_marker(
            StatusCode::CREATED,
            br#"{"id":"whep_1","secret":"whsec_x"}"#,
        ))
        .unwrap();
        assert_eq!(marker["idempotent_replay"], "response_not_kept");
        assert_eq!(marker["status"], 201);
        assert_eq!(marker["id"], "whep_1");
        assert!(!marker.to_string().contains("whsec_x"));
        let unnamed: IdempotentReplayMarker =
            serde_json::from_slice(&too_large_marker(StatusCode::OK, b"[1, 2]")).unwrap();
        assert_eq!(unnamed.id, None);
    }

    #[test]
    fn a_response_kept_without_its_content_type_replays_as_json() {
        let replay = replayed(200, None, br#"{"id":"plan_1"}"#.to_vec());
        assert_eq!(
            replay.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }

    #[test]
    fn bodies_alike_but_for_key_order_fingerprint_alike() {
        let a = fingerprint(
            &Method::POST,
            "/api/v2/plans",
            br#"{"a":1,"b":{"c":2,"d":3}}"#,
        );
        let b = fingerprint(
            &Method::POST,
            "/api/v2/plans",
            br#"{"b":{"d":3,"c":2},"a":1}"#,
        );
        let other = fingerprint(
            &Method::POST,
            "/api/v2/products",
            br#"{"a":1,"b":{"c":2,"d":3}}"#,
        );
        assert_eq!(a.unwrap(), b.unwrap());
        assert_ne!(
            fingerprint(&Method::POST, "/api/v2/plans", br#"{"a":1}"#).unwrap(),
            other.unwrap()
        );
    }
}
