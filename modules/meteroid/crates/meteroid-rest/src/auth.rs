use common_grpc::middleware::common::auth::BEARER_AUTH_HEADER;

use crate::api_key_permissions::{Requirement, requirement};
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
use common_grpc::middleware::server::auth::{AuthorizedAsTenant, TenantActor, TenantEnv};
use http::request::Parts;
use http::{HeaderMap, Request};
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_store::Store;
use meteroid_store::domain::api_key_permissions::ApiKeyPermissions;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::OrganizationsInterface;
use meteroid_store::repositories::api_tokens::{ApiTokensInterface, is_publishable_key};
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
/// reach them, even under an allowed prefix. API v2 names them alike.
const CUSTOMER_MONEY_ROUTES: &[&str] = &["top-ups", "credit-grants", "budgets"];

pub fn is_customer_money_route(method: &http::Method, path: &str) -> bool {
    let Some(rest) = path
        .strip_prefix("/api/v1/customers/")
        .or_else(|| path.strip_prefix("/api/v2/customers/"))
    else {
        return false;
    };
    let mut segments = rest.split('/').skip(1);
    match segments.next() {
        Some(route) if CUSTOMER_MONEY_ROUTES.contains(&route) => true,
        Some("wallets") => !matches!(*method, http::Method::GET | http::Method::HEAD),
        _ => false,
    }
}

/// The customer money routes, plus the invoice writes that record, reverse or write off a payment,
/// and issuing a credit note (it refunds or credits money back).
pub(crate) fn is_restricted_money_route(method: &http::Method, path: &str) -> bool {
    if is_customer_money_route(method, path) {
        return true;
    }
    if *method != http::Method::POST {
        return false;
    }
    if path.trim_end_matches('/') == "/api/v2/credit-notes" {
        return true;
    }
    let Some((_, action)) = path
        .strip_prefix("/api/v1/invoices/")
        .or_else(|| path.strip_prefix("/api/v2/invoices/"))
        .and_then(|rest| rest.split_once('/'))
    else {
        return false;
    };
    matches!(action, "payments" | "mark-paid" | "mark-uncollectible")
        || (action.starts_with("payments/") && action.ends_with("/reverse"))
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
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    let path = req.uri().path();

    if !path.starts_with("/api/") {
        return Ok(next.run(req).await);
    }

    // OAuth endpoints are public (token exchange, introspection, revocation
    // authenticate via client credentials in the request body / Basic auth), in v1 as in v2.
    if path.starts_with("/api/v1/oauth/") || path.starts_with(crate::v2::OAUTH_PREFIX) {
        return Ok(next.run(req).await);
    }

    // The client API authenticates with a customer portal token or, on its public routes, a
    // publishable key (`client_auth_middleware`); never with a secret key.
    if path.starts_with(CLIENT_API_PREFIX) {
        return Ok(next.run(req).await);
    }

    let (mut parts, body) = req.into_parts();
    authenticate(&store, delegated.as_deref(), &mut parts)
        .await
        .map_err(challenged)?;
    Ok(next.run(Request::from_parts(parts, body)).await)
}

/// A `401` names the scheme the API authenticates with (RFC 9110 §11.6.1): `WWW-Authenticate:
/// Bearer`, unless it names its own (OAuth's client authentication).
pub fn challenged(mut response: Response) -> Response {
    if response.status() == StatusCode::UNAUTHORIZED
        && !response
            .headers()
            .contains_key(http::header::WWW_AUTHENTICATE)
    {
        response.headers_mut().insert(
            http::header::WWW_AUTHENTICATE,
            http::HeaderValue::from_static("Bearer"),
        );
    }
    response
}

/// Checks the credentials and stores who the caller is acting as in the request extensions.
async fn authenticate(
    store: &Store,
    delegated: Option<&dyn RestDelegatedAuth>,
    req: &mut Parts,
) -> Result<(), Response> {
    let path = req.uri.path();

    let bearer = req
        .headers
        .get(BEARER_AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_string);
    if let (Some(token), Some(delegated)) = (bearer, delegated)
        && delegated.claims_token(&token)
    {
        delegated.check_token_path(path).map_err(delegated_error)?;
        if is_restricted_money_route(&req.method, path) {
            return Err(money_route_forbidden());
        }
        let granted = delegated
            .authenticate_token(store, &token, req)
            .await
            .map_err(|err| {
                log::debug!("Failed to validate a delegated token: {err:?}");
                delegated_error(err)
            })?;
        req.extensions.insert(granted.authorized);
        req.extensions.insert(granted.reach);
        req.extensions.insert(DelegatedCredentials);
        return Ok(());
    }

    let validated = validate_api_key(&req.headers, store).await.map_err(|err| {
        log::debug!("Failed to validate API key: {err:?}");
        let json = Json(RestErrorResponse {
            code: ErrorCode::Unauthorized,
            message: err.msg.unwrap_or_else(|| "Unauthorized".to_string()),
        });
        (err.status, json).into_response()
    })?;

    let ValidatedKey {
        id,
        resolved:
            ResolvedKey {
                organization_id,
                tenant_id,
                tenant_env,
                permissions,
                expires_at,
            },
    } = validated;
    if expires_at.is_some_and(|at| at <= common_clock::wall().naive_utc()) {
        let json = Json(RestErrorResponse {
            code: ErrorCode::Unauthorized,
            message: "This API key has expired".to_string(),
        });
        return Err((StatusCode::UNAUTHORIZED, json).into_response());
    }
    check_key_permissions(permissions.as_ref(), &req.method, path)?;

    // Check for Connect Account header
    // When present, resolve to the child org's tenant instead of the platform's.
    if req.headers.contains_key(CONNECT_ACCOUNT_HEADER) {
        if is_restricted_money_route(&req.method, path) {
            return Err(money_route_forbidden());
        }
        let Some(delegated) = delegated else {
            return Err(delegated_error(DelegatedAuthError {
                status: StatusCode::FORBIDDEN,
                message: "Acting for a connected account is not available".to_string(),
            }));
        };
        let granted = delegated
            .act_for_account(store, organization_id, id, req)
            .await
            .map_err(|err| {
                log::debug!("Failed to resolve connect account: {err:?}");
                delegated_error(err)
            })?;

        req.extensions.insert(granted.authorized);
        req.extensions.insert(granted.reach);
        req.extensions.insert(DelegatedCredentials);
        return Ok(());
    }

    // Check express org restrictions for direct API key access
    if is_organization_express_cached(store, organization_id)
        .await
        .map_err(|err| {
            let json = Json(RestErrorResponse {
                code: ErrorCode::Unauthorized,
                message: err.msg.unwrap_or_else(|| "Unauthorized".to_string()),
            });
            (err.status, json).into_response()
        })?
    {
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
        if is_restricted_money_route(&req.method, path) {
            return Err(money_route_forbidden());
        }
        req.extensions.insert(DelegatedCredentials);
    }

    let state = AuthorizedAsTenant {
        tenant_id,
        organization_id,
        actor: TenantActor::ApiKey(id),
        tenant_env,
    };
    req.extensions.insert(state);
    Ok(())
}

fn check_key_permissions(
    permissions: Option<&ApiKeyPermissions>,
    method: &http::Method,
    path: &str,
) -> Result<(), Response> {
    let Some(permissions) = permissions else {
        return Ok(());
    };
    let message = match requirement(method, path) {
        Requirement::AnyKey => return Ok(()),
        Requirement::Resource { resource, write } if permissions.allows(resource, write) => {
            return Ok(());
        }
        Requirement::Resource { resource, write } => format!(
            "This API key has no {} access to {resource}",
            if write { "write" } else { "read" }
        ),
        Requirement::AllWrite(resources)
            if resources.iter().all(|r| permissions.allows(*r, true)) =>
        {
            return Ok(());
        }
        Requirement::AllWrite(resources) => format!(
            "This endpoint needs write access to {}",
            resources
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Requirement::Unrestricted => "This endpoint needs an API key with full access".to_string(),
    };
    let json = Json(RestErrorResponse {
        code: ErrorCode::Forbidden,
        message,
    });
    Err((StatusCode::FORBIDDEN, json).into_response())
}

/// A secret key the store resolved and whose secret matched.
#[derive(Clone)]
struct ResolvedKey {
    organization_id: OrganizationId,
    tenant_id: TenantId,
    tenant_env: TenantEnv,
    permissions: Option<ApiKeyPermissions>,
    expires_at: Option<chrono::NaiveDateTime>,
}

struct ValidatedKey {
    id: ApiTokenId,
    resolved: ResolvedKey,
}

#[derive(Debug)]
struct AuthStatus {
    status: StatusCode,
    msg: Option<String>,
}

/// A tenant changed (its environment, say): its keys' cached environments may be stale.
/// `None` means changes may have been missed, so every key is forgotten.
pub async fn forget_api_tokens(tenant: Option<TenantId>) {
    use cached::Cached;
    let mut cache = VALIDATE_API_TOKEN_BY_ID_CACHED.lock().await;
    let Some(tenant) = tenant else {
        cache.cache_clear();
        return;
    };
    let stale: Vec<_> = cache
        .key_order()
        .zip(cache.value_order())
        .filter(|(_, (_, resolved))| resolved.tenant_id == tenant)
        .map(|(key, _)| *key)
        .collect();
    for key in stale {
        cache.cache_remove(&key);
    }
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
) -> Result<ResolvedKey, AuthStatus> {
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

    Ok(ResolvedKey {
        organization_id: res.organization_id,
        tenant_id: res.tenant_id,
        tenant_env,
        permissions: res.permissions,
        expires_at: res.expires_at,
    })
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
) -> Result<ValidatedKey, AuthStatus> {
    let api_key = header_map
        .get(BEARER_AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some("Invalid or missing Authorization header".to_string()),
        })?;

    // A publishable key reads the client API's public routes only. Its row would not validate
    // anyway (secret keys only, and it has no hash); refused here before any lookup.
    if is_publishable_key(api_key) {
        return Err(AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some(
                "A publishable key only reads the client API's public routes; use a secret key"
                    .to_string(),
            ),
        });
    }

    let (validator, id) = ApiTokenValidator::parse_api_key(api_key)
        .and_then(|v| {
            v.extract_identifier()
                .map(|id| (v, ApiTokenId::from_const(id)))
        })
        .map_err(|_| AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some("Invalid API key format".to_string()),
        })?;

    let resolved = validate_api_token_by_id_cached(store, &validator, &id).await?;

    Ok(ValidatedKey { id, resolved })
}

#[cfg(test)]
mod tests {
    use super::{is_customer_money_route, is_restricted_money_route};
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
            (Method::POST, "/api/v2/customers/ext:acme/top-ups"),
            (Method::POST, "/api/v2/customers/cus_1/credit-grants"),
            (Method::PATCH, "/api/v2/customers/ext:acme/wallets/EUR"),
            (Method::GET, "/api/v2/customers/ext:acme/budgets"),
            (
                Method::POST,
                "/api/v2/customers/ext:acme/budgets/bdgt_1/archive",
            ),
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
            (Method::GET, "/api/v2/customers/ext:acme/wallets/check"),
            (
                Method::GET,
                "/api/v2/customers/ext:acme/entitlements/api/check",
            ),
        ] {
            assert!(!is_customer_money_route(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn invoice_payment_writes_require_direct_credentials() {
        for path in [
            "/api/v1/invoices/inv_1/mark-paid",
            "/api/v1/invoices/inv_1/payments",
            "/api/v1/invoices/inv_1/payments/tx_1/reverse",
            "/api/v1/invoices/inv_1/mark-uncollectible",
            "/api/v2/invoices/inv_1/mark-paid",
            "/api/v2/invoices/inv_1/payments",
            "/api/v2/invoices/inv_1/payments/pay_1/reverse",
            "/api/v2/credit-notes",
        ] {
            assert!(is_restricted_money_route(&Method::POST, path), "{path}");
        }
        for (method, path) in [
            (Method::GET, "/api/v2/credit-notes"),
            (Method::POST, "/api/v2/invoices"),
            (Method::GET, "/api/v1/invoices/inv_1"),
            (Method::POST, "/api/v1/invoices/inv_1/refresh"),
            (Method::PATCH, "/api/v1/invoices/inv_1/custom-properties"),
        ] {
            assert!(!is_restricted_money_route(&method, path), "{method} {path}");
        }
    }
}
