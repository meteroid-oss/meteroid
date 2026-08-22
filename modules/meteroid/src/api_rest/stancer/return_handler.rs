//! Customer return-URL handler for the Stancer hosted card page.
//!
//! When `MandateOps::initiate_mandate_setup` creates the Stancer payment
//! intent, the `return_url` we hand to Stancer is shaped like:
//!
//!   `{rest_api_url}/v1/portal/stancer/return?connection={cid}&dest={dest}&intent={pi_…}`
//!
//! (`intent` is PATCHed on by the adapter once the intent id exists.) The
//! hosted page auto-redirects the customer here after the card is entered.
//!
//! Unlike GoCardless — whose completion is webhook-driven and whose return
//! handler only classifies — Stancer has NO webhooks, so this handler IS the
//! completion + money path: it calls `Services::complete_hosted_setup`,
//! which finalizes the intent (ownership-checked — the endpoint is
//! unauthenticated), saves the card, and runs the fail-closed first payment
//! (invoice charge / checkout activation). Then it bounces the customer back
//! to their original page (`dest`) with a `stancer_status` marker.
//!
//! Provider boundary: this route stays provider-named (each provider's return
//! URL is its own thin route — it owns the query shape and redirect markers),
//! but ALL completion logic lives in the generic capability-gated
//! `complete_hosted_setup` routine. A future webhook-less
//! (`HostedSetupCompletion::PollingRequired`) provider adds its own thin
//! return route here and delegates to that same routine.

use crate::api_rest::AppState;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use common_domain::ids::{BaseId, CustomerConnectionId};
use meteroid_store::services::HostedSetupOutcome;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ReturnQuery {
    /// Globally-unique v7 UUID (base62) identifying the customer connection.
    /// Attacker-supplied; `complete_hosted_setup` ownership-checks the
    /// intent's metadata against it before attaching or charging anything.
    pub connection: String,
    /// The Stancer payment intent id (`pi_…`) to complete, baked into the
    /// return URL by the adapter at setup time.
    pub intent: Option<String>,
    /// The customer's original page (e.g. the invoice-payment URL, which
    /// carries its own auth token). We bounce them back here. Validated
    /// against the portal origin before use, to prevent an open redirect.
    pub dest: Option<String>,
}

/// Allowlist for the intent id before we hand it to the completion service:
/// Stancer ids are short alphanumeric tokens (`pi_…`, `paym_…`); anything
/// outside this class is refused up-front.
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

    // Completion is the money path here (no webhook backstop): finalize the
    // intent, save the card, and run the fail-closed first payment.
    let outcome = app_state
        .services
        .complete_hosted_setup(connection_id, intent_id)
        .await;

    // Outcome vocabulary the frontend consumes from the redirect:
    //   ok             — card saved; any named invoice charge / checkout
    //                    activation was initiated.
    //   processing     — the intent has no card yet; refresh to retry.
    //   payment_failed — card saved, but the first charge was declined; offer
    //                    a retry with the saved card.
    //   failed         — the hosted flow ended without a saved card, or an
    //                    internal error occurred.
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
            // The provider decline code is NOT reflected into the URL — it is
            // provider-controlled text; the page re-fetches details over the
            // authenticated API instead.
            redirect_back(&dest, &[("stancer_status", "payment_failed")])
        }
        Ok(HostedSetupOutcome::SetupFailed) => {
            log::info!("Stancer setup failed for connection {connection_id}");
            redirect_back(&dest, &[("stancer_status", "failed")])
        }
        Ok(HostedSetupOutcome::HeldForReview { .. }) => {
            // Money WAS captured but could not be reconciled onto the checkout
            // transaction (mismatch / cancelled-row race). The store already
            // logged the manual-review error; the customer must not be told to
            // retry (a retry would double-charge) — surface as processing.
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
/// query string. Values here are our own controlled tokens.
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
