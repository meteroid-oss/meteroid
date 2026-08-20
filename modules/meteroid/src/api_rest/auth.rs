use common_grpc::middleware::common::auth::BEARER_AUTH_HEADER;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, Request};
use tracing::log;

use crate::api_rest::error::{ErrorCode, RestErrorResponse};
use common_grpc::middleware::server::auth::{AuthorizedAsTenant, TenantActor};
use meteroid_middleware::server::auth::api_key::{ApiKeyAuthError, VerifiedApiKey, verify_api_key};
use meteroid_store::Store;

pub async fn auth_middleware(
    State(store): State<Store>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    if !req.uri().path().starts_with("/api/") {
        return Ok(next.run(req).await);
    }

    let verified = authenticate(req.headers(), &store).await.map_err(|err| {
        log::debug!("Failed to validate API key: {err:?}");
        unauthorized(match err {
            ApiKeyAuthError::Malformed => "Invalid or missing Authorization header",
            ApiKeyAuthError::Unauthorized => "Unauthorized",
        })
    })?;

    req.extensions_mut().insert(AuthorizedAsTenant {
        tenant_id: verified.tenant_id,
        organization_id: verified.organization_id,
        actor: TenantActor::ApiKey(verified.id),
        tenant_env: verified.tenant_env,
    });

    Ok(next.run(req).await)
}

fn unauthorized(message: &str) -> Response {
    let json = Json(RestErrorResponse {
        code: ErrorCode::Unauthorized,
        message: message.to_string(),
    });

    (StatusCode::UNAUTHORIZED, json).into_response()
}

async fn authenticate(
    header_map: &HeaderMap,
    store: &Store,
) -> Result<VerifiedApiKey, ApiKeyAuthError> {
    let api_key = header_map
        .get(BEARER_AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(ApiKeyAuthError::Malformed)?;

    verify_api_key(api_key, store).await
}
