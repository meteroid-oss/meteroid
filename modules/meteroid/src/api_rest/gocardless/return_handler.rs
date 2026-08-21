//! Customer return-URL handler for the GoCardless Billing Request Flow.
//!
//! When `MandateOps::initiate_mandate_setup` creates a Billing Request Flow,
//! the `redirect_uri` we hand to GoCardless is shaped like:
//!
//!   `{rest_api_url}/v1/portal/gocardless/return?connection={cid}&dest={dest}`
//!
//! GoCardless redirects the customer to this URL verbatim on completion. The
//! mandate attach + any invoice charge are NOT done here: they are driven by the
//! `billing_requests.fulfilled` webhook (the hosted flow auto-fulfils
//! asynchronously, so completing synchronously here would race fulfillment).
//! This handler only bounces the customer back to their original page (`dest`)
//! in a "processing" state, so it can never fail the money path.
//!
//! GoCardless can also redirect here with `?error=<code>` if the customer
//! abandoned the flow. We sanitize that code (alphanumerics + `_-`, max 64
//! chars) before reflecting it back into the redirect URL.

use crate::api_rest::AppState;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use common_domain::ids::{BaseId, CustomerConnectionId};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ReturnQuery {
    /// Globally-unique v7 UUID identifying the customer connection. Logged for
    /// traceability; the mandate attach + any invoice charge are driven by the
    /// `billing_requests.fulfilled` webhook, not this handler.
    pub connection: String,
    /// GoCardless can also redirect here with an error (rare).
    pub error: Option<String>,
    /// The customer's original page (e.g. the invoice-payment URL, which carries
    /// its own auth token). We bounce them back here. Validated against the
    /// portal origin before use, to prevent an open redirect.
    pub dest: Option<String>,
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

/// GoCardless exit_uri marker (set by the adapter) meaning the customer bailed
/// out of the hosted flow via GoCardless's own Cancel/Exit — a graceful
/// abandonment, not an error. (Browser "back" doesn't hit this handler at all.)
const ABANDONED_MARKER: &str = "flow_abandoned";

#[axum::debug_handler]
pub async fn handle(Query(q): Query<ReturnQuery>, State(app_state): State<AppState>) -> Response {
    // The connection id is informational here. GoCardless auto-fulfils the
    // Billing Request asynchronously, so at redirect time the mandate may not
    // exist yet — completing/charging synchronously would RACE fulfillment.
    // The `billing_requests.fulfilled` webhook is the single source of truth: it
    // attaches the mandate and (for an invoice setup) charges it. This handler
    // only classifies the outcome and bounces the customer back to `dest`.
    let connection_id = CustomerConnectionId::parse_base62(&q.connection)
        .map(|c| c.as_base62())
        .unwrap_or_else(|_| "<invalid>".to_string());

    let dest = safe_dest(&app_state, q.dest.as_deref());

    // Outcome vocabulary the frontend (`consumeGocardlessReturn`) understands:
    //   ok        — authorised; the webhook attaches the mandate and charges.
    //   abandoned — customer cancelled inside GoCardless; offer the form again.
    //   failed    — a real error; show it and offer the form again.
    match q.error.as_deref() {
        None => {
            log::info!(
                "GoCardless customer authorised for connection {connection_id}; mandate + charge via webhook"
            );
            redirect_back(&dest, &[("gocardless_status", "ok")])
        }
        Some(ABANDONED_MARKER) => {
            log::info!("GoCardless flow abandoned for connection {connection_id}");
            redirect_back(&dest, &[("gocardless_status", "abandoned")])
        }
        Some(err) => {
            let code = sanitize_error_code(err);
            log::info!("GoCardless return error for connection {connection_id}: {code}");
            redirect_back(
                &dest,
                &[("gocardless_status", "failed"), ("gocardless_error", code)],
            )
        }
    }
}

/// Resolve the post-completion destination. Only honour a caller-supplied `dest`
/// if it points at our own portal origin (open-redirect defense); otherwise fall
/// back to the customer portal root. A bare `starts_with(portal)` is NOT enough:
/// `https://portal.acme.com.evil.com/…` has the origin as a prefix. We require
/// the prefix to end at an origin boundary (end-of-string, or the next char
/// begins the path/query/fragment).
fn safe_dest(app_state: &AppState, dest: Option<&str>) -> String {
    let portal = app_state.portal_url.trim_end_matches('/');
    let same_origin = |d: &str| {
        d.strip_prefix(portal).is_some_and(|rest| {
            rest.is_empty()
                || rest.starts_with('/')
                || rest.starts_with('?')
                || rest.starts_with('#')
        })
    };
    match dest {
        Some(d) if same_origin(d) => d.to_string(),
        _ => format!("{portal}/portal/customer"),
    }
}

/// Append `key=value` params to `dest`, respecting whether it already has a
/// query string. Values here are our own controlled tokens / sanitized codes.
fn redirect_back(dest: &str, params: &[(&str, &str)]) -> Response {
    let mut url = dest.to_string();
    let mut has_query = dest.contains('?');
    for (key, value) in params {
        url.push(if has_query { '&' } else { '?' });
        url.push_str(&format!("{key}={value}"));
        has_query = true;
    }
    Redirect::to(&url).into_response()
}
