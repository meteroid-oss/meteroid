//! **Temporary bridge** between the new normalized outcomes
//! ([`ChargeOutcome`]) and the legacy [`PaymentIntent`] domain type used by
//! the settlement repository ([`consolidate_intent_and_transaction_tx`]).
//!
//! Once Step 4 refactors the settlement layer to consume normalized events
//! directly, this module goes away. Until then, service callers build the
//! legacy struct here so no behavior changes downstream.
//!
//! [`consolidate_intent_and_transaction_tx`]: crate::repositories::payment_transactions::PaymentTransactionInterface::consolidate_intent_and_transaction_tx

use super::events::NormalizedEventKind;
use super::model::ChargeOutcome;
use crate::domain::PaymentStatusEnum;
use crate::domain::payment_transactions::PaymentIntent;
use common_domain::ids::{PaymentTransactionId, TenantId};

/// Build the legacy [`PaymentIntent`] from a normalized [`ChargeOutcome`].
///
/// `transaction_id`, `tenant_id`, `amount_minor`, and `currency` are caller-
/// supplied because [`ChargeOutcome`] does not carry them (they belong to the
/// charge *request*, not the outcome).
pub fn payment_intent_from_outcome(
    outcome: ChargeOutcome,
    transaction_id: PaymentTransactionId,
    tenant_id: TenantId,
    amount_minor: i64,
    currency: String,
) -> PaymentIntent {
    match outcome {
        ChargeOutcome::Succeeded(r) => PaymentIntent {
            external_id: r.external_id,
            transaction_id,
            tenant_id,
            amount_requested: amount_minor,
            amount_received: Some(r.amount_received_minor),
            currency,
            next_action: None,
            status: PaymentStatusEnum::Settled,
            last_payment_error: None,
            processed_at: Some(r.processed_at),
        },
        ChargeOutcome::Pending(a) => PaymentIntent {
            external_id: a.external_id,
            transaction_id,
            tenant_id,
            amount_requested: amount_minor,
            amount_received: None,
            currency,
            next_action: None,
            status: PaymentStatusEnum::Pending,
            last_payment_error: None,
            processed_at: None,
        },
        ChargeOutcome::RequiresAction(action) => {
            // Until Step 4 surfaces `RequiresAction` to the customer portal,
            // keep parity with the old behaviour: leave the transaction
            // Pending and log a hint. The customer will be prompted to
            // re-authorize via a follow-up flow.
            let (external_id, next_action) = match action {
                super::model::RequiresActionInstruction::HostedUrl { external_id, url, .. } => {
                    (external_id, Some(url))
                }
                super::model::RequiresActionInstruction::ClientSecret { external_id, .. } => {
                    (external_id, Some("client-side-action".to_string()))
                }
            };
            PaymentIntent {
                external_id,
                transaction_id,
                tenant_id,
                amount_requested: amount_minor,
                amount_received: None,
                currency,
                next_action,
                status: PaymentStatusEnum::Pending,
                last_payment_error: None,
                processed_at: None,
            }
        }
        ChargeOutcome::Cancelled(c) => PaymentIntent {
            external_id: c.external_id.unwrap_or_default(),
            transaction_id,
            tenant_id,
            amount_requested: amount_minor,
            amount_received: None,
            currency,
            next_action: None,
            status: PaymentStatusEnum::Cancelled,
            last_payment_error: Some(c.message),
            processed_at: None,
        },
        ChargeOutcome::Failed(f) => PaymentIntent {
            external_id: f.external_id.unwrap_or_default(),
            transaction_id,
            tenant_id,
            amount_requested: amount_minor,
            amount_received: None,
            currency,
            next_action: None,
            status: PaymentStatusEnum::Failed,
            last_payment_error: Some(f.message),
            processed_at: None,
        },
    }
}

/// Build a legacy [`PaymentIntent`] from a [`NormalizedEventKind`] (used by
/// the webhook router to feed [`consolidate_intent_and_transaction_tx`]).
///
/// The local `transaction_id` and `tenant_id` are looked up by the caller
/// using `meteroid_transaction_id` metadata; this helper just maps the event
/// payload to the legacy struct shape.
///
/// **Important**: the settlement function only reads `status`, `processed_at`,
/// and `last_payment_error` from the intent — it does NOT write `amount`,
/// `amount_received`, or `currency` back to the DB. So the placeholder values
/// (zero amount, empty currency) below for non-success kinds are
/// intentionally "don't care" — they never reach storage. The `amount_received`
/// from a real `PaymentSucceeded` event would otherwise be the right field
/// to surface once consolidation is widened.
///
/// Returns `None` for kinds that don't map to a transaction settlement
/// (`PaymentMethodAttached`, disputes, …). The router handles those out of band.
///
/// [`consolidate_intent_and_transaction_tx`]: crate::repositories::payment_transactions::PaymentTransactionInterface::consolidate_intent_and_transaction_tx
pub fn payment_intent_from_event(
    kind: &NormalizedEventKind,
    transaction_id: PaymentTransactionId,
    tenant_id: TenantId,
) -> Option<PaymentIntent> {
    match kind {
        NormalizedEventKind::PaymentSucceeded(e) => Some(PaymentIntent {
            external_id: e.external_transaction_id.clone(),
            transaction_id,
            tenant_id,
            amount_requested: e.amount_received_minor,
            amount_received: Some(e.amount_received_minor),
            currency: e.currency.clone(),
            next_action: None,
            status: PaymentStatusEnum::Settled,
            last_payment_error: None,
            processed_at: Some(chrono::Utc::now().naive_utc()),
        }),
        NormalizedEventKind::PaymentFailed(e) => Some(PaymentIntent {
            external_id: e.external_transaction_id.clone(),
            transaction_id,
            tenant_id,
            amount_requested: 0,
            amount_received: None,
            currency: String::new(),
            next_action: None,
            status: PaymentStatusEnum::Failed,
            last_payment_error: Some(e.message.clone()),
            processed_at: None,
        }),
        NormalizedEventKind::PaymentPending(e) => Some(PaymentIntent {
            external_id: e.external_transaction_id.clone(),
            transaction_id,
            tenant_id,
            amount_requested: 0,
            amount_received: None,
            currency: String::new(),
            next_action: None,
            status: PaymentStatusEnum::Pending,
            last_payment_error: None,
            processed_at: None,
        }),
        _ => None,
    }
}
