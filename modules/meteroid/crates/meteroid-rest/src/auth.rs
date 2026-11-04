use common_grpc::middleware::common::auth::BEARER_AUTH_HEADER;

use crate::client::CLIENT_API_PREFIX;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cached::proc_macro::cached;
use common_domain::ids::{ApiTokenId, OrganizationId, TenantId};
use common_grpc::middleware::server::auth::api_token_validator::{
    ApiTokenSecretDigest, ApiTokenValidator,
};
use common_grpc::middleware::server::auth::{
    AuthenticatedState, AuthorizedAsTenant, TenantActor, TenantEnv,
};
use http::{HeaderMap, Request};
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_store::Store;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::OrganizationsInterface;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use tracing::{error, log};

/// Header of a platform's API key acting for one of its connected accounts.
pub use meteroid_rest_common::delegated_auth::{
    CONNECT_ACCOUNT_HEADER, DelegatedAuthError, RestDelegatedAuth,
};

#[derive(Clone)]
pub struct AuthState {
    pub store: Store,
    pub delegated: Option<std::sync::Arc<dyn RestDelegatedAuth>>,
}

fn delegated_error(err: DelegatedAuthError) -> Response {
    let json = Json(RestErrorResponse {
        code: ErrorCode::Unauthorized,
        message: err.message,
    });
    (err.status, json).into_response()
}

/// REST API paths that express organizations ARE allowed to access.
/// Everything else under /api/ is blocked for express orgs.
const EXPRESS_ALLOWED_PATH_PREFIXES: &[&str] = &["/api/v1/customers", "/api/v1/invoices"];

/// Customer routes that charge a card, grant credit or set spending controls. Delegated access
/// (Connect tokens, a platform acting for a connected account) and express organizations never
/// reach them, even under an allowed prefix.
const CUSTOMER_MONEY_ROUTES: &[&str] = &["top-ups", "credit-grants", "budgets"];

pub fn is_customer_money_route(method: &http::Method, path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/api/v1/customers/") else {
        return false;
    };
    let mut segments = rest.split('/').skip(1);
    match segments.next() {
        Some(route) if CUSTOMER_MONEY_ROUTES.contains(&route) => true,
        Some("wallets") => !matches!(*method, http::Method::GET | http::Method::HEAD),
        _ => false,
    }
}

/// Marks delegated or express requests, so the portal sessions they open inherit the deny.
pub use common_grpc::middleware::server::auth::DelegatedCredentials;

fn money_route_forbidden() -> Response {
    let json = Json(RestErrorResponse {
        code: ErrorCode::Forbidden,
        message: "This endpoint is not accessible with delegated or express credentials"
            .to_string(),
    });
    (StatusCode::FORBIDDEN, json).into_response()
}

pub async fn auth_middleware(
    State(AuthState { store, delegated }): State<AuthState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    let path = req.uri().path();

    if !path.starts_with("/api/") {
        return Ok(next.run(req).await);
    }

    // OAuth endpoints are public (token exchange, introspection, revocation
    // authenticate via client credentials in the request body / Basic auth).
    if path.starts_with("/api/v1/oauth/") {
        return Ok(next.run(req).await);
    }

    // The client API authenticates with a customer portal token (`client_auth_middleware`).
    if path.starts_with(CLIENT_API_PREFIX) {
        return Ok(next.run(req).await);
    }

    let bearer = req
        .headers()
        .get(BEARER_AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_string);
    if let (Some(token), Some(delegated)) = (bearer, delegated.as_ref())
        && delegated.claims_token(&token)
    {
        let path = req.uri().path();
        delegated.check_token_path(path).map_err(delegated_error)?;
        if is_customer_money_route(req.method(), path) {
            return Err(money_route_forbidden());
        }
        let authorized = delegated
            .authenticate_token(&store, &token, req.headers())
            .await
            .map_err(|err| {
                log::debug!("Failed to validate a delegated token: {err:?}");
                delegated_error(err)
            })?;
        req.extensions_mut().insert(authorized);
        req.extensions_mut().insert(DelegatedCredentials);
        return Ok(next.run(req).await);
    }

    // Handle API key
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
        tenant_env,
    } = authenticated_state
    {
        // Check for Connect Account header
        // When present, resolve to the child org's tenant instead of the platform's.
        if req.headers().contains_key(CONNECT_ACCOUNT_HEADER) {
            if is_customer_money_route(req.method(), req.uri().path()) {
                return Err(money_route_forbidden());
            }
            let Some(delegated) = delegated.as_ref() else {
                return Err(delegated_error(DelegatedAuthError {
                    status: StatusCode::FORBIDDEN,
                    message: "Acting for a connected account is not available".to_string(),
                }));
            };
            let authorized = delegated
                .act_for_account(&store, organization_id, id, req.headers())
                .await
                .map_err(|err| {
                    log::debug!("Failed to resolve connect account: {err:?}");
                    delegated_error(err)
                })?;

            req.extensions_mut().insert(authorized);
            req.extensions_mut().insert(DelegatedCredentials);
            return Ok(next.run(req).await);
        }

        // Check express org restrictions for direct API key access
        if is_organization_express_cached(&store, organization_id)
            .await
            .map_err(|err| {
                let json = Json(RestErrorResponse {
                    code: ErrorCode::Unauthorized,
                    message: err.msg.unwrap_or_else(|| "Unauthorized".to_string()),
                });
                (err.status, json).into_response()
            })?
        {
            let path = req.uri().path();
            if !EXPRESS_ALLOWED_PATH_PREFIXES
                .iter()
                .any(|prefix| path.starts_with(prefix))
            {
                let json = Json(RestErrorResponse {
                    code: ErrorCode::Unauthorized,
                    message: "This endpoint is not accessible by express organizations".to_string(),
                });
                return Err((StatusCode::FORBIDDEN, json).into_response());
            }
            if is_customer_money_route(req.method(), path) {
                return Err(money_route_forbidden());
            }
            req.extensions_mut().insert(DelegatedCredentials);
        }

        let state = AuthorizedAsTenant {
            tenant_id,
            organization_id,
            actor: TenantActor::ApiKey(id),
            tenant_env,
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

#[derive(Debug)]
struct AuthStatus {
    status: StatusCode,
    msg: Option<String>,
}

// The key must include the presented secret: keying on the token id alone lets anyone
// quoting a known id reuse the entry a legitimate request populated, skipping validate_hash.
#[cached(
    result = true,
    size = 50_000,
    time = 120, // 2 min
    key = "(ApiTokenId, ApiTokenSecretDigest)",
    convert = r#"{ (*api_key_id, validator.secret_digest()) }"#
)]
async fn validate_api_token_by_id_cached(
    store: &Store,
    validator: &ApiTokenValidator,
    api_key_id: &ApiTokenId,
) -> Result<(OrganizationId, TenantId, TenantEnv), AuthStatus> {
    let res = store
        .get_api_token_by_id_for_validation(api_key_id)
        .await
        .map_err(|err| {
            match err.current_context() {
                StoreError::ValueNotFound(_) => {}
                other => {
                    error!("Failed to resolve api key: {:?}", other);
                }
            }
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

    // Fire-and-forget usage stamp; debounced by this fn's cache TTL.
    {
        let store = store.clone();
        let api_key_id = *api_key_id;
        tokio::spawn(async move {
            if let Err(err) = store.report_api_token_usage(&api_key_id).await {
                error!("Failed to record api token usage for {api_key_id}: {err:?}");
            }
        });
    }

    use meteroid_store::domain::enums::TenantEnvironmentEnum;
    let tenant_env = if res.environment == TenantEnvironmentEnum::Production {
        TenantEnv::Production
    } else {
        TenantEnv::NonProduction
    };

    Ok((res.organization_id, res.tenant_id, tenant_env))
}

#[cached(
    result = true,
    size = 100,
    time = 300, // 5 min
    key = "OrganizationId",
    convert = r#"{ org_id }"#
)]
async fn is_organization_express_cached(
    store: &Store,
    org_id: OrganizationId,
) -> Result<bool, AuthStatus> {
    let org = store
        .get_organization_by_id(org_id)
        .await
        .map_err(|_| AuthStatus {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            msg: Some("Failed to resolve organization".to_string()),
        })?;
    Ok(org.is_express)
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
        .and_then(|v| {
            v.extract_identifier()
                .map(|id| (v, ApiTokenId::from_const(id)))
        })
        .map_err(|_| AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some("Invalid API key format".to_string()),
        })?;

    let (organization_id, tenant_id, tenant_env) =
        validate_api_token_by_id_cached(store, &validator, &id).await?;

    Ok(AuthenticatedState::ApiKey {
        id,
        tenant_id,
        organization_id,
        tenant_env,
    })
}

#[cfg(test)]
mod tests {
    use super::is_customer_money_route;
    use http::Method;

    #[test]
    fn customer_money_routes_are_recognized_whatever_the_customer_reference() {
        for (method, path) in [
            (Method::POST, "/api/v1/customers/cus_1/top-ups"),
            (Method::GET, "/api/v1/customers/acme/credit-grants"),
            (Method::POST, "/api/v1/customers/acme/credit-grants"),
            (Method::PATCH, "/api/v1/customers/acme/wallets/EUR"),
            (Method::GET, "/api/v1/customers/acme/budgets"),
            (Method::DELETE, "/api/v1/customers/acme/budgets/bdgt_1"),
        ] {
            assert!(is_customer_money_route(&method, path), "{method} {path}");
        }
        for (method, path) in [
            (Method::GET, "/api/v1/customers/acme/wallets"),
            (Method::GET, "/api/v1/customers/acme/wallets/check"),
            (Method::GET, "/api/v1/customers/acme/credit-ledger"),
            (Method::GET, "/api/v1/customers/acme"),
            (Method::POST, "/api/v1/customers"),
            (Method::GET, "/api/v1/invoices/inv_1"),
            (Method::GET, "/api/v1/customers/top-ups"),
        ] {
            assert!(!is_customer_money_route(&method, path), "{method} {path}");
        }
    }
}
