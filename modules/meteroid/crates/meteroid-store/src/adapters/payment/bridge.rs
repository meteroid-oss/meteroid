//! Maps the normalized [`ChargeOutcome`] / [`NormalizedEventKind`] onto the
//! internal [`PaymentIntent`] consumed by the settlement repository
//! ([`consolidate_intent_and_transaction_tx`]) — the single provider-agnostic
//! shape both the charge and webhook paths converge to.
//!
//! [`consolidate_intent_and_transaction_tx`]: crate::repositories::payment_transactions::PaymentTransactionInterface::consolidate_intent_and_transaction_tx

use super::events::NormalizedEventKind;
use super::model::ChargeOutcome;
use crate::domain::PaymentStatusEnum;
use crate::domain::payment_transactions::{PaymentIntent, PaymentNextAction};
use common_domain::ids::{PaymentTransactionId, TenantId};
use secrecy::{ExposeSecret, SecretString};

/// Extra params are caller-supplied — [`ChargeOutcome`] carries the outcome,
/// not the originating request (amount, currency, ids).
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
            // Stays Pending; the persisted next_action is what marks it as
            // "awaiting customer authentication".
            let (external_id, next_action) = match action {
                super::model::RequiresActionInstruction::HostedUrl {
                    external_id, url, ..
                } => (external_id, PaymentNextAction::RedirectToUrl { url }),
                super::model::RequiresActionInstruction::ClientSecret {
                    external_id,
                    client_secret,
                    publishable_key,
                } => (
                    external_id.clone(),
                    PaymentNextAction::UseSdk {
                        intent_id: external_id,
                        publishable_key: publishable_key.expose_secret().to_string(),
                        client_secret: Some(SecretString::from(client_secret)),
                    },
                ),
            };
            PaymentIntent {
                external_id,
                transaction_id,
                tenant_id,
                amount_requested: amount_minor,
                amount_received: None,
                currency,
                next_action: Some(next_action),
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

/// `None` for kinds that don't settle a transaction (mandate/dispute events,
/// handled elsewhere). The zero amount / empty currency on failed/pending are
/// unused — consolidation reads only status, processed_at, last_payment_error.
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
