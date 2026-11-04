use crate::AppState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Redirect, Response};
use common_domain::ids::SsoConnectionId;
use error_stack::Report;
use meteroid_store::domain::org_security_audit_log::SecurityRequestContext;
use meteroid_store::domain::users::LoginUserResponse;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::SsoInterface;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct DiscoverParams {
    email: String,
}

#[derive(Serialize)]
pub struct DiscoverResponse {
    connection_id: String,
    provider: String,
    // Whether this org mandates SSO; lets the login UI hide the password path.
    enforced: bool,
}

/// Resolve which SSO connection (if any) owns an email's domain, so the login
/// screen can offer "Sign in with SSO". Intentionally leaks nothing beyond the
/// opaque connection id, protocol, and whether SSO is enforced.
#[axum::debug_handler]
pub async fn discover(
    Query(params): Query<DiscoverParams>,
    State(app_state): State<AppState>,
) -> Response {
    match app_state
        .store
        .find_sso_connection_for_email(&params.email)
        .await
    {
        Ok(Some(conn)) => Json(DiscoverResponse {
            connection_id: conn.id.as_proto(),
            provider: conn.provider.to_string(),
            enforced: conn.enforced,
        })
        .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            log::warn!("SSO discover failed: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct AuthorizeParams {
    connection_id: String,
    // Present when SSO was started from an invite link, so the callback can
    // consume the invite and land the member with the invited role.
    invite_key: Option<String>,
}

/// Kick off the OIDC authorization-code flow: redirect the browser to the
/// organization's identity provider.
#[axum::debug_handler]
pub async fn authorize(
    Query(params): Query<AuthorizeParams>,
    State(app_state): State<AppState>,
) -> Redirect {
    let Ok(connection_id) = SsoConnectionId::from_proto(&params.connection_id) else {
        return Redirect::to(&login_error(
            &app_state.portal_url,
            "invalid SSO connection",
        ));
    };
    let invite_key = params.invite_key.filter(|k| !k.trim().is_empty());

    match app_state
        .store
        .sso_authorize_url(
            connection_id,
            invite_key,
            sso_callback_url(&app_state.rest_api_external_url),
        )
        .await
    {
        Ok(url) => Redirect::to(url.expose_secret()),
        Err(e) => {
            log::warn!("SSO authorize failed: {e:?}");
            Redirect::to(&signin_error_url(&app_state.portal_url, e))
        }
    }
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: String,
    state: String,
}

/// IdP redirect target: exchange the code and issue a session (or MFA
/// challenge), reusing the same success/challenge landing pages as social OAuth.
#[axum::debug_handler]
pub async fn callback(
    Query(params): Query<CallbackParams>,
    headers: HeaderMap,
    State(app_state): State<AppState>,
) -> Redirect {
    let request_context = security_context_from_headers(&headers, app_state.trusted_proxy_hops);

    match app_state
        .store
        .sso_signin(
            params.code.into(),
            params.state.into(),
            sso_callback_url(&app_state.rest_api_external_url),
            request_context,
        )
        .await
    {
        Ok(LoginUserResponse::Authenticated { token, .. }) => {
            Redirect::to(signin_success_url(&app_state.portal_url, &token).as_str())
        }
        Ok(LoginUserResponse::MfaChallenge { challenge_token }) => {
            Redirect::to(mfa_challenge_url(&app_state.portal_url, &challenge_token).as_str())
        }
        Err(e) => {
            log::warn!("SSO callback failed: {e:?}");
            Redirect::to(signin_error_url(&app_state.portal_url, e).as_str())
        }
    }
}

/// The SSO redirect URI, registered with each IdP. Single static path — the
/// specific connection is recovered from the opaque `state`, not the URL.
fn sso_callback_url(rest_api_external_url: &str) -> String {
    format!("{rest_api_external_url}/sso/callback")
}

fn signin_success_url(public_url: &str, token: &SecretString) -> String {
    format!(
        "{}/oauth_success?token={}",
        public_url,
        token.expose_secret()
    )
}

fn mfa_challenge_url(public_url: &str, challenge_token: &SecretString) -> String {
    format!(
        "{}/mfa-challenge?token={}",
        public_url,
        challenge_token.expose_secret()
    )
}

fn login_error(public_url: &str, message: &str) -> String {
    format!("{}/login?error={}", public_url, message)
}

fn signin_error_url(public_url: &str, error: Report<StoreError>) -> String {
    let message = match error.current_context() {
        StoreError::OauthError(m) => m.as_str(),
        StoreError::LoginError(m) => m.as_str(),
        _ => "SSO sign-in failed",
    };
    login_error(public_url, message)
}

/// Mirrors the OAuth router: the SSO callback hits axum (not tonic), so we
/// extract the client IP / user agent here and delegate parsing to the shared
/// security-context helper.
fn security_context_from_headers(
    headers: &HeaderMap,
    trusted_proxy_hops: usize,
) -> SecurityRequestContext {
    use meteroid_app::security_context::{REAL_IP_HEADER, XFF_HEADER, client_ip_from_xff};

    let ip_address = headers
        .get(XFF_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| client_ip_from_xff(s, trusted_proxy_hops))
        .or_else(|| {
            headers
                .get(REAL_IP_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });

    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    meteroid_app::security_context::build(ip_address, user_agent)
}
