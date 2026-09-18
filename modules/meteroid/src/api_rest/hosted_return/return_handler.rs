//! Customer return-URL handler for all hosted-redirect providers. The provider's capabilities
//! decide whether the return completes the setup or only classifies it. The customer is
//! redirected to `dest` with `hosted_status` (and `hosted_error` on failure).

use crate::api_rest::AppState;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use common_domain::ids::{BaseId, CustomerConnectionId};
use meteroid_store::adapters::payment::events::{NormalizedEventKind, NormalizedWebhookEvent};
use meteroid_store::domain::connectors::Connector;
use meteroid_store::services::HostedReturnOutcome;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ReturnQuery {
    /// Caller-supplied: completion checks it against the intent's metadata.
    pub connection: String,
    /// Provider intent to complete, put in the return URL by the adapter. Absent when the webhook
    /// is the only completion path.
    #[serde(alias = "payment")]
    pub intent: Option<String>,
    /// `flow_abandoned` if the customer cancelled on the hosted page; otherwise a provider error
    /// code.
    pub error: Option<String>,
    /// Validated against the portal origin (open-redirect guard).
    pub dest: Option<String>,
}

/// Exit marker every adapter sets on its cancel URL.
const ABANDONED_MARKER: &str = "flow_abandoned";

const STATUS: &str = "hosted_status";
const ERROR: &str = "hosted_error";

type Params = Vec<(&'static str, String)>;

/// Outcome decided from the query alone, before any completion runs.
#[derive(Debug, PartialEq, Eq)]
enum ReturnRequest {
    Complete {
        connection_id: CustomerConnectionId,
        intent_id: Option<String>,
    },
    /// Redirect back immediately with these params.
    Settled(Params),
}

/// Provider ids and error codes are short tokens; anything else is rejected before it can be
/// reflected into the redirect URL.
fn is_safe_token(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 64
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn params(status: &str, error: Option<&str>) -> Params {
    let mut out = vec![(STATUS, status.to_string())];
    if let Some(code) = error {
        out.push((ERROR, code.to_string()));
    }
    out
}

fn classify(q: &ReturnQuery) -> ReturnRequest {
    let Ok(connection_id) = CustomerConnectionId::parse_base62(&q.connection) else {
        return ReturnRequest::Settled(params("failed", Some("invalid_request")));
    };
    match q.error.as_deref() {
        None => {}
        Some(ABANDONED_MARKER) => return ReturnRequest::Settled(params("abandoned", None)),
        Some(code) => {
            let code = if is_safe_token(code) {
                code
            } else {
                "unknown_error"
            };
            return ReturnRequest::Settled(params("failed", Some(code)));
        }
    }
    match q.intent.as_deref() {
        Some(raw) if !is_safe_token(raw) => {
            ReturnRequest::Settled(params("failed", Some("invalid_request")))
        }
        intent => ReturnRequest::Complete {
            connection_id,
            intent_id: intent.map(str::to_string),
        },
    }
}

/// Params to redirect back with per outcome. A decline code is kept out of the URL; the page
/// fetches it via the authenticated API.
fn outcome_params(outcome: &HostedReturnOutcome) -> Params {
    match outcome {
        HostedReturnOutcome::Completed => params("ok", None),
        HostedReturnOutcome::Processing => params("processing", None),
        HostedReturnOutcome::PaymentFailed => params("payment_failed", None),
        HostedReturnOutcome::Abandoned => params("abandoned", None),
        HostedReturnOutcome::SetupFailed { .. } => params("failed", None),
        HostedReturnOutcome::MissingIntent => params("failed", Some("invalid_request")),
    }
}

#[axum::debug_handler]
pub async fn handle(Query(q): Query<ReturnQuery>, State(app_state): State<AppState>) -> Response {
    let dest = safe_dest(&app_state, q.dest.as_deref());

    let (connection_id, intent_id) = match classify(&q) {
        ReturnRequest::Settled(params) => {
            log::info!("Hosted return settled before completion: {params:?}");
            return redirect_back(&dest, &params);
        }
        ReturnRequest::Complete {
            connection_id,
            intent_id,
        } => (connection_id, intent_id),
    };

    let outcome = match app_state
        .services
        .complete_hosted_return(connection_id, intent_id.clone())
        .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            log::error!("Hosted setup errored for connection {connection_id}: {e:?}");
            return redirect_back(&dest, &params("failed", Some("internal_error")));
        }
    };
    let params = outcome_params(&outcome);
    log::info!("Hosted return for connection {connection_id} settled as {params:?}");
    if let (
        HostedReturnOutcome::SetupFailed {
            connector: Some(connector),
        },
        Some(intent_id),
    ) = (&outcome, &intent_id)
    {
        apply_final_state(&app_state, connector, intent_id).await;
    }
    redirect_back(&dest, &params)
}

/// Re-reads a failed webhook-backed intent now (the webhook repeats it idempotently) so an
/// invoice payment shows as failed on return, not pending.
async fn apply_final_state(app_state: &AppState, connector: &Connector, intent_id: &str) {
    let applied = match meteroid_store::adapters::payment::initialize_payment_connector(connector) {
        Ok(connector_impl) => crate::api_rest::webhooks::event_handler::handle_normalized_event(
            NormalizedWebhookEvent {
                provider_event_id: format!("return:{intent_id}"),
                provider_event_type: "hosted_return.changed".to_string(),
                occurred_at: chrono::Utc::now(),
                kind: NormalizedEventKind::ResourceChanged {
                    resource_ref: intent_id.to_string(),
                },
                owner_tenant_id: None,
            },
            connector,
            connector_impl.as_ref(),
            app_state.store.clone(),
            &app_state.services,
        )
        .await
        .map_err(|e| format!("{e:?}")),
        Err(e) => Err(format!("{e:?}")),
    };
    if let Err(e) = applied {
        log::warn!("Hosted return: intent {intent_id} state not applied (the webhook will): {e}");
    }
}

/// Open-redirect guard: `dest` must start with the portal origin and end there at a boundary
/// (`https://portal.acme.com.evil.com/…` has the origin as a prefix).
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

/// Appends `key=value` params to `dest`. Values are our own tokens.
fn return_url(dest: &str, params: &[(&'static str, String)]) -> String {
    let mut url = dest.to_string();
    let mut has_query = dest.contains('?');
    for (key, value) in params {
        url.push(if has_query { '&' } else { '?' });
        url.push_str(&format!("{key}={value}"));
        has_query = true;
    }
    url
}

fn redirect_back(dest: &str, params: &[(&'static str, String)]) -> Response {
    Redirect::to(&return_url(dest, params)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(connection: &str, intent: Option<&str>, error: Option<&str>) -> ReturnQuery {
        ReturnQuery {
            connection: connection.to_string(),
            intent: intent.map(str::to_string),
            error: error.map(str::to_string),
            dest: None,
        }
    }

    fn pairs(p: &Params) -> Vec<(&'static str, &str)> {
        p.iter().map(|(k, v)| (*k, v.as_str())).collect()
    }

    fn settled(req: ReturnRequest) -> Vec<(&'static str, String)> {
        match req {
            ReturnRequest::Settled(p) => p,
            other => panic!("expected a settled return, got {other:?}"),
        }
    }

    #[test]
    fn classifies_before_completion() {
        let conn_id = CustomerConnectionId::new();
        let connection = conn_id.as_base62();
        #[allow(non_snake_case)]
        let connection = connection.as_str();
        assert_eq!(
            classify(&query(connection, None, None)),
            ReturnRequest::Complete {
                connection_id: conn_id,
                intent_id: None
            }
        );
        assert_eq!(
            classify(&query(connection, Some("paym_1"), None)),
            ReturnRequest::Complete {
                connection_id: conn_id,
                intent_id: Some("paym_1".to_string())
            }
        );
        assert_eq!(
            pairs(&settled(classify(&query(
                connection,
                None,
                Some("flow_abandoned")
            )))),
            [("hosted_status", "abandoned")]
        );
        assert_eq!(
            pairs(&settled(classify(&query(
                connection,
                Some("paym_1"),
                Some("card_declined")
            )))),
            [
                ("hosted_status", "failed"),
                ("hosted_error", "card_declined")
            ]
        );
        assert_eq!(
            pairs(&settled(classify(&query(
                connection,
                None,
                Some("<script>")
            )))),
            [
                ("hosted_status", "failed"),
                ("hosted_error", "unknown_error")
            ]
        );
        assert_eq!(
            pairs(&settled(classify(&query(connection, Some("a/../b"), None)))),
            [
                ("hosted_status", "failed"),
                ("hosted_error", "invalid_request")
            ]
        );
        assert_eq!(
            pairs(&settled(classify(&query(
                "not-an-id",
                Some("paym_1"),
                None
            )))),
            [
                ("hosted_status", "failed"),
                ("hosted_error", "invalid_request")
            ]
        );
    }

    #[test]
    fn outcome_markers() {
        let cases = [
            (HostedReturnOutcome::Completed, "ok", None),
            (HostedReturnOutcome::Processing, "processing", None),
            (HostedReturnOutcome::PaymentFailed, "payment_failed", None),
            (HostedReturnOutcome::Abandoned, "abandoned", None),
            (
                HostedReturnOutcome::SetupFailed { connector: None },
                "failed",
                None,
            ),
            (
                HostedReturnOutcome::MissingIntent,
                "failed",
                Some("invalid_request"),
            ),
        ];
        for (outcome, status, error) in cases {
            assert_eq!(
                outcome_params(&outcome),
                params(status, error),
                "{outcome:?}"
            );
        }
    }

    #[test]
    fn return_url_appends_to_existing_query() {
        let p = params("failed", Some("invalid_request"));
        assert_eq!(
            return_url("https://portal.acme.com/portal/pay?x=1", &p),
            "https://portal.acme.com/portal/pay?x=1&hosted_status=failed&hosted_error=invalid_request"
        );
        assert_eq!(
            return_url("https://portal.acme.com/portal/pay", &params("ok", None)),
            "https://portal.acme.com/portal/pay?hosted_status=ok"
        );
    }

    #[test]
    fn token_allowlist() {
        assert!(is_safe_token("pi_abc123XYZ"));
        assert!(is_safe_token("tr_7UhSN1zuXS"));
        assert!(is_safe_token("paym_9-x_Y"));
        assert!(!is_safe_token(""));
        assert!(!is_safe_token("tr_abc/../etc"));
        assert!(!is_safe_token("pi_<script>"));
        assert!(!is_safe_token(&"a".repeat(65)));
    }
}
