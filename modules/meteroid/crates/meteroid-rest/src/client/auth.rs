use crate::AppState;
use crate::client::{
    AuthorizedAsCustomer, AuthorizedAsPublishable, CLIENT_API_PREFIX, OriginRefused,
    is_public_route,
};
use crate::v2::problem::{Problem, ProblemCode};
use axum::extract::State;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cached::proc_macro::cached;
use common_domain::auth::PortalScope;
use common_domain::ids::TenantId;
use common_grpc::middleware::common::auth::BEARER_AUTH_HEADER;
use common_grpc::middleware::server::auth::TenantEnv;
use http::{HeaderMap, Request, StatusCode};
use meteroid_store::Store;
use meteroid_store::domain::PublishableKey;
use meteroid_store::domain::enums::TenantEnvironmentEnum;
use meteroid_store::domain::portal_tokens::{PortalTokenStatus, is_portal_token};
use meteroid_store::errors::StoreError;
use meteroid_store::jwt_claims::{ClientClaims, PortalJwtClaims, PortalTokenError, ResourceAccess};
use meteroid_store::repositories::TenantInterface;
use meteroid_store::repositories::api_tokens::{ApiTokensInterface, is_publishable_key};
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use tracing::{error, log};

/// Two kinds of routes, two credentials, never crossed: the public routes take a publishable key
/// and nothing else, every other client route a customer token and never a publishable key.
pub async fn client_auth_middleware(
    State(app_state): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    let path = req.uri().path();
    if !path.starts_with(CLIENT_API_PREFIX) {
        return Ok(next.run(req).await);
    }

    if is_public_route(req.method(), path) {
        let authorized = authorize_publishable(req.headers(), &app_state.store)
            .await
            .map_err(|rejection| {
                log::debug!("Public client API request rejected: {}", rejection.message);
                rejection.into_response()
            })?;
        req.extensions_mut().insert(authorized);
        return Ok(next.run(req).await);
    }

    let writes = writes(req.method(), path);
    let authorized = authorize(req.headers(), &app_state, writes)
        .await
        .map_err(|rejection| {
            log::debug!("Client API request rejected: {}", rejection.message);
            rejection.into_response()
        })?;

    req.extensions_mut().insert(authorized);
    Ok(next.run(req).await)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(BEARER_AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
}

/// Posts that only read, answered like a `GET` would be.
const READING_POSTS: &[&str] = &["/api/client/v1/entitlements/check"];

fn writes(method: &http::Method, path: &str) -> bool {
    !(method.is_safe() || *method == http::Method::POST && READING_POSTS.contains(&path))
}

async fn authorize(
    headers: &HeaderMap,
    app_state: &AppState,
    writes: bool,
) -> Result<AuthorizedAsCustomer, Rejection> {
    let token = bearer(headers).ok_or(Rejection::unauthorized(
        "Invalid or missing Authorization header",
    ))?;

    // Neither a portal token nor a JWT, it would be refused below anyway: said plainly.
    if is_publishable_key(token) {
        return Err(Rejection::unauthorized(
            "A publishable key only reads the public routes; this one takes a customer token",
        ));
    }

    let (tenant_id, resource, scopes, claims) = if is_portal_token(token) {
        stored_token(token, app_state).await?
    } else {
        jwt_token(token, app_state)?
    };

    let ResourceAccess::Customer(customer_id) = resource else {
        return Err(Rejection::forbidden("Not a customer token"));
    };

    if !grants_read(scopes.as_deref()) || writes && !grants_manage(scopes.as_deref()) {
        return Err(Rejection::forbidden(
            "The token's scopes do not allow this operation.",
        ));
    }

    let tenant_env = resolve_tenant_env(app_state, tenant_id).await?;

    Ok(AuthorizedAsCustomer {
        tenant_id,
        customer_id,
        scopes,
        tenant_env,
        claims,
    })
}

/// A public route reads the tenant's public data: a publishable key names the tenant, and its
/// origins, when it lists any, are the only sites a browser may read it from. A request without
/// `Origin` is not a browser's cross-site one and is not held to them: an origin is no secret, it
/// keeps another site from embedding the key, not anyone from reading what is public.
async fn authorize_publishable(
    headers: &HeaderMap,
    store: &Store,
) -> Result<AuthorizedAsPublishable, Rejection> {
    let key = bearer(headers).ok_or(Rejection::unauthorized(
        "A publishable key (pk_…) is required as a bearer token",
    ))?;
    if !is_publishable_key(key) {
        // A secret key is never accepted where a browser sends credentials.
        return Err(Rejection::unauthorized(
            "This route takes a publishable key (pk_…), never a secret key or a customer token",
        ));
    }

    let publishable = publishable_key_cached(store, key)
        .await?
        .ok_or(Rejection::unauthorized("Invalid publishable key"))?;

    if let Some(origin) = headers.get(http::header::ORIGIN)
        && !publishable.allowed_origins.is_empty()
    {
        let origin = origin.to_str().unwrap_or_default().trim_end_matches('/');
        if !publishable
            .allowed_origins
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(origin))
        {
            return Err(Rejection::origin_refused());
        }
    }

    Ok(AuthorizedAsPublishable {
        tenant_id: publishable.tenant_id,
        key_id: publishable.id,
        tenant_env: match publishable.environment {
            TenantEnvironmentEnum::Production => TenantEnv::Production,
            _ => TenantEnv::NonProduction,
        },
    })
}

/// A revoked key keeps working until its entry expires, a minute at most. The cache is keyed on
/// the whole key, which is public; an unknown key is cached too, so a flood of them reads the
/// database once each.
#[cached(
    result = true,
    size = 10_000,
    time = 60,
    key = "String",
    convert = r#"{ presented.to_string() }"#
)]
async fn publishable_key_cached(
    store: &Store,
    presented: &str,
) -> Result<Option<PublishableKey>, Rejection> {
    let found = store
        .authenticate_publishable_key(presented)
        .await
        .map_err(|err| {
            error!("Failed to resolve a publishable key: {err:?}");
            Rejection::internal()
        })?;

    // Fire-and-forget usage stamp; debounced by this fn's cache TTL.
    if let Some(found) = &found {
        let store = store.clone();
        let id = found.id;
        tokio::spawn(async move {
            if let Err(err) = store.report_api_token_usage(&id).await {
                error!("Failed to record publishable key usage for {id}: {err:?}");
            }
        });
    }

    Ok(found)
}

type TokenGrant = (
    TenantId,
    ResourceAccess,
    Option<Vec<PortalScope>>,
    ClientClaims,
);

async fn stored_token(token: &str, app_state: &AppState) -> Result<TokenGrant, Rejection> {
    let portal_token = app_state
        .store
        .authenticate_portal_token(token)
        .await
        .map_err(|err| {
            error!("Failed to resolve portal token: {err:?}");
            Rejection::internal()
        })?
        .ok_or(Rejection::unauthorized("Invalid token"))?;

    match portal_token.status(common_clock::wall()) {
        PortalTokenStatus::Active => {}
        PortalTokenStatus::Expired => return Err(Rejection::expired()),
        PortalTokenStatus::Revoked => return Err(Rejection::unauthorized("Invalid token")),
    }
    Ok((
        portal_token.tenant_id,
        portal_token.resource,
        portal_token.scopes,
        portal_token.client,
    ))
}

/// SDK tokens minted before stored portal tokens.
fn jwt_token(token: &str, app_state: &AppState) -> Result<TokenGrant, Rejection> {
    let claims =
        PortalJwtClaims::decode(token, &app_state.jwt_secret).map_err(|err| match err {
            PortalTokenError::Expired => Rejection::expired(),
            PortalTokenError::Invalid => Rejection::unauthorized("Invalid token"),
        })?;

    if claims.exp.is_none() {
        return Err(Rejection::unauthorized("Token has no expiry"));
    }
    Ok((
        claims.tenant_id,
        claims.resource,
        claims.scopes,
        claims.client,
    ))
}

/// `manage` implies `read`, and a legacy token without scopes has full access.
fn grants_read(scopes: Option<&[PortalScope]>) -> bool {
    scopes.is_none_or(|scopes| {
        scopes
            .iter()
            .any(|scope| matches!(scope, PortalScope::Read | PortalScope::Manage))
    })
}

fn grants_manage(scopes: Option<&[PortalScope]>) -> bool {
    scopes.is_none_or(|scopes| scopes.contains(&PortalScope::Manage))
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
                Rejection::internal()
            }
        })?;

    Ok(match tenant.tenant.environment {
        TenantEnvironmentEnum::Production => TenantEnv::Production,
        _ => TenantEnv::NonProduction,
    })
}

/// A refusal of the client API, answered as a problem (the envelope adds the request id).
#[derive(Clone, Debug)]
struct Rejection {
    status: StatusCode,
    code: ProblemCode,
    message: &'static str,
    /// The browser's origin is not one the key lists: no CORS header may let it read the answer.
    origin_refused: bool,
}

impl Rejection {
    fn new(status: StatusCode, code: ProblemCode, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
            origin_refused: false,
        }
    }

    fn unauthorized(message: &'static str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, ProblemCode::Unauthorized, message)
    }

    fn expired() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ProblemCode::TokenExpired,
            "Token expired",
        )
    }

    fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ProblemCode::InternalError,
            "Internal server error",
        )
    }

    fn forbidden(message: &'static str) -> Self {
        Self::new(StatusCode::FORBIDDEN, ProblemCode::Forbidden, message)
    }

    fn origin_refused() -> Self {
        Self {
            origin_refused: true,
            ..Self::forbidden("This publishable key is not accepted from this origin")
        }
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        let mut response = Problem::new(self.status, self.code, self.message).into_response();
        if self.origin_refused {
            response.extensions_mut().insert(OriginRefused);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Method;

    #[test]
    fn only_manage_writes() {
        let cancel = "/api/client/v1/retention/sessions";
        assert!(writes(&Method::POST, cancel));
        assert!(writes(&Method::DELETE, "/api/client/v1/customer"));
        assert!(!writes(&Method::GET, "/api/client/v1/customer"));
        assert!(!writes(&Method::POST, "/api/client/v1/entitlements/check"));

        assert!(grants_manage(None));
        assert!(grants_manage(Some(&[PortalScope::Manage])));
        assert!(grants_read(Some(&[PortalScope::Read])));
        assert!(!grants_manage(Some(&[PortalScope::Read])));
        let card = [PortalScope::PaymentMethods];
        assert!(
            !grants_read(Some(&card)),
            "a card link reaches nothing here"
        );
        assert!(!grants_manage(Some(&card)));
    }
}
