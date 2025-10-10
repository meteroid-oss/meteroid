use common_grpc::middleware::common::auth::BEARER_AUTH_HEADER;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cached::proc_macro::cached;
use common_grpc::middleware::server::auth::api_token_validator::ApiTokenValidator;
use http::{HeaderMap, Request};
use tracing::{error, log};

use crate::api_rest::error::{ErrorCode, RestErrorResponse};
use common_domain::ids::{OrganizationId, TenantId};
use common_grpc::middleware::server::auth::{AuthenticatedState, AuthorizedAsTenant};
use meteroid_store::Store;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use std::time::Duration;
use uuid::Uuid;

pub async fn auth_middleware(
    State(store): State<Store>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    if !req.uri().path().starts_with("/api/") {
        return Ok(next.run(req).await);
    }

    let authenticated_state = validate_api_key(req.headers(), &store)
        .await
        .map_err(|err| {
            log::debug!("Failed to validate API key: {err:?}");
            let json = Json(RestErrorResponse {
                code: ErrorCode::Unauthorized,
                message: err.msg.unwrap_or_else(|| "Unauthorized".to_string()),
            });
            (err.status, json).into_response()
        })?;

    if let AuthenticatedState::ApiKey {
        tenant_id,
        id,
        organization_id,
    } = authenticated_state
    {
        let state = AuthorizedAsTenant {
            tenant_id,
            organization_id,
            actor_id: id,
        };
        req.extensions_mut().insert(state);
        return Ok(next.run(req).await);
    }

    let err = Json(RestErrorResponse {
        code: ErrorCode::Unauthorized,
        message: "Unauthorized".to_string(),
    });

    Err((StatusCode::UNAUTHORIZED, err).into_response())
}

#[allow(unused)]
#[derive(Debug)]
struct AuthStatus {
    status: StatusCode,
    msg: Option<String>,
}

#[cached(
    result = true,
    size = 100,
    time = 120, // 2 min
    key = "Uuid",
    convert = r#"{ *api_key_id }"#
)]
async fn validate_api_token_by_id_cached(
    store: &Store,
    validator: &ApiTokenValidator,
    api_key_id: &Uuid,
) -> Result<(OrganizationId, TenantId), AuthStatus> {
    let res = store
        .get_api_token_by_id_for_validation(api_key_id)
        .await
        .map_err(|err| {
            error!("Failed to resolve api key: {:?}", err);
            AuthStatus {
                status: StatusCode::UNAUTHORIZED,
                msg: Some("Failed to resolve api key".to_string()),
            }
        })?;

    validator
        .validate_hash(&res.hash)
        .map_err(|_e| AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some("Unauthorized. Invalid hash".to_string()),
        })?;

    Ok((res.organization_id, res.tenant_id))
}

async fn validate_api_key(
    header_map: &HeaderMap,
    store: &Store,
) -> Result<AuthenticatedState, AuthStatus> {
    let api_key = header_map
        .get(BEARER_AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some("Invalid or missing Authorization header".to_string()),
        })?;

    let (validator, id) = ApiTokenValidator::parse_api_key(api_key)
        .and_then(|v| v.extract_identifier().map(|id| (v, id)))
        .map_err(|_| AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some("Invalid API key format".to_string()),
        })?;

    let (organization_id, tenant_id) =
        validate_api_token_by_id_cached(store, &validator, &id).await?;

    Ok(AuthenticatedState::ApiKey {
        id,
        tenant_id,
        organization_id,
    })
}
