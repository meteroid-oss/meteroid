//! Customer return-URL handler for the GoCardless Billing Request Flow.
//!
//! When `MandateOps::initiate_mandate_setup` creates a Billing Request Flow,
//! the `redirect_uri` we hand to GoCardless is shaped like:
//!
//!   `{rest_api_url}/v1/portal/gocardless/return?connection={cid}`
//!
//! GoCardless appends its own `billing_request` query parameter when it
//! redirects the customer back. This handler resolves the connection, calls
//! the service layer to complete the Billing Request and store the resulting
//! mandate, then 302-redirects the customer back into the portal.
//!
//! Unauthenticated (post third-party redirect) and `connection` is
//! attacker-controllable, so `Services::complete_gocardless_setup`
//! ownership-checks the completed BR's metadata before attaching. GC's
//! `complete` action is idempotent, so replay is harmless.
//!
//! GoCardless can also redirect here with `?error=<code>` if the customer
//! abandoned the flow. We sanitize that code (alphanumerics + `_-`, max 64
//! chars) before reflecting it back into the redirect URL.

use crate::api_rest::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use common_domain::ids::{BaseId, CustomerConnectionId};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ReturnQuery {
    /// Globally-unique v7 UUID identifying the customer connection. The
    /// tenant is read from the connection row server-side.
    pub connection: String,
    /// GoCardless appends this on redirect.
    pub billing_request: Option<String>,
    /// GoCardless can also redirect here with an error (rare).
    pub error: Option<String>,
}

/// Allowlist of characters we accept in the GC-provided `error` query param
/// before reflecting it back into our redirect URL. GoCardless documents
/// short snake_case codes; anything outside this character class is replaced
/// with `unknown_error` so a future GC behaviour change can't inject content
/// into the URL bar the customer sees.
fn sanitize_error_code(raw: &str) -> &str {
    if raw.len() <= 64
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        raw
    } else {
        "unknown_error"
    }
}

#[axum::debug_handler]
pub async fn handle(
    Query(q): Query<ReturnQuery>,
    State(app_state): State<AppState>,
) -> Response {
    let connection_id = match CustomerConnectionId::parse_base62(&q.connection) {
        Ok(c) => c,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid connection id"),
    };

    if let Some(err) = q.error.as_deref() {
        log::info!(
            "GoCardless return signalled error for connection {connection_id}: {err}"
        );
        return Redirect::to(&format!(
            "{}/portal/customer?gocardless_error={}",
            app_state.portal_url.trim_end_matches('/'),
            sanitize_error_code(err)
        ))
        .into_response();
    }

    let billing_request_id = match q.billing_request {
        Some(id) => id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "missing billing_request parameter",
            );
        }
    };

    match app_state
        .services
        .complete_gocardless_setup(connection_id, billing_request_id)
        .await
    {
        Ok(pm) => {
            log::info!(
                "GoCardless mandate {} attached for connection {}",
                pm.id,
                connection_id
            );
            Redirect::to(&format!(
                "{}/portal/customer?gocardless_status=ok",
                app_state.portal_url.trim_end_matches('/')
            ))
            .into_response()
        }
        Err(e) => {
            log::error!("GoCardless return completion failed: {e:?}");
            Redirect::to(&format!(
                "{}/portal/customer?gocardless_status=failed",
                app_state.portal_url.trim_end_matches('/')
            ))
            .into_response()
        }
    }
}

fn error_response(status: StatusCode, msg: &'static str) -> Response {
    (status, msg).into_response()
}
