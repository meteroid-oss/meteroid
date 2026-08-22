//! Customer return-URL handler for the Stancer hosted card page:
//! `{rest_api_url}/v1/portal/stancer/return?connection={cid}&dest={dest}&intent={pi_…}`.
//! Stancer has NO webhooks, so this handler IS the completion + money path:
//! `complete_hosted_setup` finalizes the intent (ownership-checked — the
//! endpoint is unauthenticated), saves the card, runs the fail-closed first
//! payment, then bounces the customer back to `dest` with a `stancer_status`
//! marker. The route is provider-named; all completion logic is generic.

use crate::api_rest::AppState;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use common_domain::ids::{BaseId, CustomerConnectionId};
use meteroid_store::services::HostedSetupOutcome;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ReturnQuery {
    /// Customer connection id (base62). Attacker-supplied;
    /// `complete_hosted_setup` ownership-checks the intent's metadata against it.
    pub connection: String,
    /// The Stancer payment intent id to complete, baked into the return URL.
    pub intent: Option<String>,
    /// The customer's original page to bounce back to. Validated against the
    /// portal origin to prevent an open redirect.
    pub dest: Option<String>,
}

/// Allowlist for the intent id: Stancer ids are short alphanumeric tokens;
/// anything outside this class is refused up-front.
fn valid_intent_id(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 64
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[axum::debug_handler]
pub async fn handle(Query(q): Query<ReturnQuery>, State(app_state): State<AppState>) -> Response {
    let dest = safe_dest(&app_state, q.dest.as_deref());

    let connection_id = match CustomerConnectionId::parse_base62(&q.connection) {
        Ok(id) => id,
        Err(_) => {
            log::info!("Stancer return with invalid connection id");
            return redirect_back(
                &dest,
                &[
                    ("stancer_status", "failed"),
                    ("stancer_error", "invalid_request"),
                ],
            );
        }
    };

    let intent_id = match q.intent.as_deref().filter(|i| valid_intent_id(i)) {
        Some(intent) => intent.to_string(),
        None => {
            log::info!("Stancer return for connection {connection_id} without a valid intent id");
            return redirect_back(
                &dest,
                &[
                    ("stancer_status", "failed"),
                    ("stancer_error", "invalid_request"),
                ],
            );
        }
    };

    let outcome = app_state
        .services
        .complete_hosted_setup(connection_id, intent_id)
        .await;

    // Redirect vocabulary the frontend consumes: ok / processing /
    // payment_failed (card saved, charge declined) / failed.
    match outcome {
        Ok(HostedSetupOutcome::MethodSaved(_)) => {
            log::info!("Stancer setup completed for connection {connection_id}: method saved");
            redirect_back(&dest, &[("stancer_status", "ok")])
        }
        Ok(HostedSetupOutcome::InvoiceCharged(_)) => {
            log::info!(
                "Stancer setup completed for connection {connection_id}: invoice charge initiated"
            );
            redirect_back(&dest, &[("stancer_status", "ok")])
        }
        Ok(HostedSetupOutcome::CheckoutActivated(_)) => {
            log::info!(
                "Stancer setup completed for connection {connection_id}: checkout activated"
            );
            redirect_back(&dest, &[("stancer_status", "ok")])
        }
        Ok(HostedSetupOutcome::Processing) => {
            log::info!("Stancer setup for connection {connection_id} still processing");
            redirect_back(&dest, &[("stancer_status", "processing")])
        }
        Ok(HostedSetupOutcome::PaymentFailed { code, .. }) => {
            log::info!(
                "Stancer setup for connection {connection_id}: card saved, first charge declined ({code:?})"
            );
            // The decline code is NOT reflected into the URL (provider-controlled
            // text); the page re-fetches details over the authenticated API.
            redirect_back(&dest, &[("stancer_status", "payment_failed")])
        }
        Ok(HostedSetupOutcome::SetupFailed) => {
            log::info!("Stancer setup failed for connection {connection_id}");
            redirect_back(&dest, &[("stancer_status", "failed")])
        }
        Ok(HostedSetupOutcome::HeldForReview { .. }) => {
            // Money WAS captured but could not be reconciled. The customer
            // must not be told to retry (a retry would double-charge) —
            // surface as processing.
            log::error!(
                "Stancer setup for connection {connection_id}: captured payment held for manual review"
            );
            redirect_back(&dest, &[("stancer_status", "processing")])
        }
        Err(e) => {
            log::error!("Stancer setup errored for connection {connection_id}: {e:?}");
            redirect_back(
                &dest,
                &[
                    ("stancer_status", "failed"),
                    ("stancer_error", "internal_error"),
                ],
            )
        }
    }
}

/// Open-redirect defense: only honour a `dest` on our own portal origin. A
/// bare `starts_with(portal)` is NOT enough (`https://portal.acme.com.evil.com/…`)
/// — the prefix must end at an origin boundary (end, or path/query/fragment).
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

/// Append `key=value` params to `dest`. Values are our own controlled tokens.
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

#[cfg(test)]
mod tests {
    use super::valid_intent_id;

    #[test]
    fn intent_id_allowlist() {
        assert!(valid_intent_id("pi_abc123XYZ"));
        assert!(valid_intent_id("paym_9-x_Y"));
        assert!(!valid_intent_id(""));
        assert!(!valid_intent_id("pi_abc/../etc"));
        assert!(!valid_intent_id("pi_<script>"));
        assert!(!valid_intent_id(&"a".repeat(65)));
    }
}
