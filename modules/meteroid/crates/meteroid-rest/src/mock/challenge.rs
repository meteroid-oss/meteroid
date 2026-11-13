//! The Test gateway's 3DS stand-in. A `requires_action` charge leaves the transaction `Pending`
//! with a next action pointing here; the customer's answer is pushed through the production
//! webhook dispatcher ([`handle_normalized_event`]). Refuses any connector that is not the mock.

use crate::AppState;
use crate::webhooks::event_handler::handle_normalized_event;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use common_domain::ids::{BaseId, ConnectorId, TenantId};
use meteroid_store::adapters::payment::events::{
    NormalizedEventKind, NormalizedWebhookEvent, PaymentFailedEvent, PaymentSucceededEvent,
};
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::enums::PaymentStatusEnum;
use meteroid_store::domain::enums::{DeclineKind, PaymentProvider};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ChallengeQuery {
    /// Tenant owning the connector, base62. Attacker-supplied; it only scopes
    /// the connector lookup — a mismatch resolves nothing.
    pub tenant: String,
    /// The mock connector the charge was made through, base62.
    pub connector: String,
    /// Provider transaction id (`mock_pi_…`) awaiting authentication.
    pub intent: String,
}

#[derive(Debug, Deserialize)]
pub struct ChallengeBody {
    /// `true` authenticates the charge, `false` fails it like a rejected 3DS.
    pub approve: bool,
}

#[derive(Debug, Serialize)]
pub struct ChallengeResponse {
    /// `settled` | `failed` | `already_resolved`
    pub status: &'static str,
}

/// Mock intent ids are `mock_pi_<amount>_<ccy>_<uuid>`; refuse anything else up front.
fn valid_intent_id(raw: &str) -> bool {
    raw.starts_with("mock_pi_")
        && raw.len() <= 96
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, message.to_string()).into_response()
}

#[axum::debug_handler]
pub async fn handle(
    Query(q): Query<ChallengeQuery>,
    State(app_state): State<AppState>,
    Json(body): Json<ChallengeBody>,
) -> Response {
    let Ok(tenant_id) = TenantId::parse_base62(&q.tenant) else {
        return bad_request("invalid tenant id");
    };
    let Ok(connector_id) = ConnectorId::parse_base62(&q.connector) else {
        return bad_request("invalid connector id");
    };
    if !valid_intent_id(&q.intent) {
        return bad_request("invalid intent id");
    }

    let connector = match app_state
        .store
        .get_connector_with_data(connector_id, tenant_id)
        .await
    {
        Ok(connector) => connector,
        Err(err) => {
            log::info!("Mock challenge for an unknown connector {connector_id}: {err:?}");
            return bad_request("unknown connector");
        }
    };

    // Never a completion path for a real provider: this endpoint settles money
    // with no signature to verify.
    if connector.payment_provider() != Some(PaymentProvider::Mock) {
        return bad_request("not a mock connector");
    }

    let transaction = match app_state
        .store
        .get_payment_tx_by_provider_transaction_id(tenant_id, &q.intent)
        .await
    {
        Ok(Some(tx)) => tx,
        Ok(None) => return (StatusCode::NOT_FOUND, "unknown intent").into_response(),
        Err(err) => {
            log::error!("Mock challenge could not load the transaction: {err:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // A challenge resolves once: a second POST (double click, replayed tab)
    // must not re-settle or overwrite a finished transaction.
    if transaction.status != PaymentStatusEnum::Pending {
        return Json(ChallengeResponse {
            status: "already_resolved",
        })
        .into_response();
    }

    // Only a charge that asked for authentication can be answered here; a plain pending payment
    // is the merchant's to settle from the backoffice.
    match app_state
        .store
        .get_payment_tx_next_action(tenant_id, transaction.id)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return bad_request("this transaction is not awaiting authentication"),
        Err(err) => {
            log::error!("Mock challenge could not read the next action: {err:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let connector_impl = match initialize_payment_connector(&connector) {
        Ok(c) => c,
        Err(err) => {
            log::error!("Mock challenge could not initialize the connector: {err:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let meteroid_transaction_id = Some(transaction.id.as_base62());
    let kind = if body.approve {
        NormalizedEventKind::PaymentSucceeded(PaymentSucceededEvent {
            external_transaction_id: q.intent.clone(),
            amount_received_minor: transaction.amount,
            currency: transaction.currency.clone(),
            meteroid_transaction_id,
        })
    } else {
        NormalizedEventKind::PaymentFailed(PaymentFailedEvent {
            external_transaction_id: q.intent.clone(),
            code: Some("authentication_failed".to_string()),
            message: "Mock 3DS challenge was declined".to_string(),
            retryable: true,
            decline_kind: DeclineKind::AuthenticationRequired,
            meteroid_transaction_id,
        })
    };

    let event = NormalizedWebhookEvent {
        provider_event_id: format!("mock_3ds_{}", q.intent),
        provider_event_type: "mock.challenge_completed".to_string(),
        occurred_at: common_domain::clock::now(),
        kind,
        owner_tenant_id: Some(tenant_id.as_base62()),
    };

    match handle_normalized_event(
        event,
        &connector,
        connector_impl.as_ref(),
        app_state.store.clone(),
        &app_state.services,
    )
    .await
    {
        Ok(()) => Json(ChallengeResponse {
            status: if body.approve { "settled" } else { "failed" },
        })
        .into_response(),
        Err(err) => {
            log::error!("Mock challenge dispatch failed for {}: {err:?}", q.intent);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::valid_intent_id;

    #[test]
    fn intent_id_allowlist() {
        assert!(valid_intent_id(
            "mock_pi_3500_EUR_0199c0ab-cdef-7000-8000-000000000000"
        ));
        assert!(!valid_intent_id("pi_stripe_123"));
        assert!(!valid_intent_id("mock_pi_<script>"));
        assert!(!valid_intent_id(&format!("mock_pi_{}", "a".repeat(96))));
    }
}
