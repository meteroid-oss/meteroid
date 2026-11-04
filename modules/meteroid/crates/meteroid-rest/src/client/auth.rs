use crate::AppState;
use crate::client::{AuthorizedAsCustomer, CLIENT_API_PREFIX};
use axum::Json;
use axum::extract::State;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use common_domain::auth::PortalScope;
use common_domain::ids::TenantId;
use common_grpc::middleware::common::auth::BEARER_AUTH_HEADER;
use common_grpc::middleware::server::auth::TenantEnv;
use http::{HeaderMap, Request, StatusCode};
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_store::domain::enums::TenantEnvironmentEnum;
use meteroid_store::errors::StoreError;
use meteroid_store::jwt_claims::{PortalJwtClaims, PortalTokenError, ResourceAccess};
use meteroid_store::repositories::TenantInterface;
use tracing::{error, log};

pub async fn client_auth_middleware(
    State(app_state): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    if !req.uri().path().starts_with(CLIENT_API_PREFIX) {
        return Ok(next.run(req).await);
    }

    let authorized = authorize(req.headers(), &app_state)
        .await
        .map_err(|rejection| {
            log::debug!("Client API request rejected: {}", rejection.message);
            rejection.into_response()
        })?;

    req.extensions_mut().insert(authorized);
    Ok(next.run(req).await)
}

async fn authorize(
    headers: &HeaderMap,
    app_state: &AppState,
) -> Result<AuthorizedAsCustomer, Rejection> {
    let token = headers
        .get(BEARER_AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(Rejection::unauthorized(
            "Invalid or missing Authorization header",
        ))?;

    let claims =
        PortalJwtClaims::decode(token, &app_state.jwt_secret).map_err(|err| match err {
            PortalTokenError::Expired => Rejection {
                status: StatusCode::UNAUTHORIZED,
                code: ErrorCode::TokenExpired,
                message: "Token expired",
            },
            PortalTokenError::Invalid => Rejection::unauthorized("Invalid token"),
        })?;

    if claims.exp.is_none() {
        return Err(Rejection::unauthorized("Token has no expiry"));
    }

    let ResourceAccess::Customer(customer_id) = claims.resource else {
        return Err(Rejection::forbidden("Not a customer token"));
    };

    if !grants_read(claims.scopes.as_deref()) {
        return Err(Rejection::forbidden(
            "The token's scopes do not allow this operation.",
        ));
    }

    let tenant_env = resolve_tenant_env(app_state, claims.tenant_id).await?;

    Ok(AuthorizedAsCustomer {
        tenant_id: claims.tenant_id,
        customer_id,
        scopes: claims.scopes,
        tenant_env,
    })
}

/// `manage` implies `read`, and a legacy token without scopes has full access.
fn grants_read(scopes: Option<&[PortalScope]>) -> bool {
    scopes.is_none_or(|scopes| {
        scopes
            .iter()
            .any(|scope| matches!(scope, PortalScope::Read | PortalScope::Manage))
    })
}

/// `find_tenant_by_id` is cached in the store.
async fn resolve_tenant_env(
    app_state: &AppState,
    tenant_id: TenantId,
) -> Result<TenantEnv, Rejection> {
    let tenant = app_state
        .store
        .find_tenant_by_id(tenant_id)
        .await
        .map_err(|err| match err.current_context() {
            StoreError::ValueNotFound(_) => Rejection::unauthorized("Invalid token"),
            _ => {
                error!("Failed to resolve tenant {tenant_id}: {err:?}");
                Rejection {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: ErrorCode::InternalServerError,
                    message: "Internal server error",
                }
            }
        })?;

    Ok(match tenant.tenant.environment {
        TenantEnvironmentEnum::Production => TenantEnv::Production,
        _ => TenantEnv::NonProduction,
    })
}

struct Rejection {
    status: StatusCode,
    code: ErrorCode,
    message: &'static str,
}

impl Rejection {
    fn unauthorized(message: &'static str) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: ErrorCode::Unauthorized,
            message,
        }
    }

    fn forbidden(message: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: ErrorCode::Forbidden,
            message,
        }
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        let body = Json(RestErrorResponse {
            code: self.code,
            message: self.message.to_string(),
        });
        (self.status, body).into_response()
    }
}
