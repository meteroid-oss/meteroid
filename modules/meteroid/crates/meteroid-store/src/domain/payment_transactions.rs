use super::enums::{DeclineKind, PaymentStatusEnum, PaymentTypeEnum, RefundMode, ReversalKind};
use chrono::NaiveDateTime;

use crate::domain::CustomerPaymentMethod;
use common_domain::ids::{
    CheckoutSessionId, CreditNoteId, CustomerConnectionId, CustomerPaymentMethodId, InvoiceId,
    PaymentTransactionId, PlanVersionId, StoredDocumentId, TenantId,
};
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionWithMethodRow};
use o2o::o2o;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// Customer action required to complete a charge (3DS / SCA). Stored in
/// `payment_transaction.next_action` (JSONB) and surfaced to the portal.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaymentNextAction {
    /// Redirect the browser to this provider-hosted URL (3DS redirect, bank app).
    RedirectToUrl { url: String },
    /// Client SDK completes the action with the intent's client secret
    /// (Stripe.js `handleNextAction`).
    UseSdk {
        intent_id: String,
        publishable_key: String,
        /// Lets the holder complete this PaymentIntent. `SecretString` keeps it
        /// out of logs; `#[serde(skip)]` keeps it out of the DB entirely (it is
        /// transient — set on a fresh charge, re-fetched from the provider when
        /// resuming).
        #[serde(skip)]
        client_secret: Option<SecretString>,
    },
}

// SecretString opts out of PartialEq; compare the non-secret identity (the
// transient secret is irrelevant to equality).
impl PartialEq for PaymentNextAction {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::RedirectToUrl { url: a }, Self::RedirectToUrl { url: b }) => a == b,
            (
                Self::UseSdk {
                    intent_id: a_id,
                    publishable_key: a_pk,
                    ..
                },
                Self::UseSdk {
                    intent_id: b_id,
                    publishable_key: b_pk,
                    ..
                },
            ) => a_id == b_id && a_pk == b_pk,
            _ => false,
        }
    }
}

/// A provider refund's state as observed on the wire or by polling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedRefundStatus {
    Succeeded,
    Pending,
    Failed,
}

#[derive(Clone, Debug, PartialEq, o2o)]
#[from_owned(PaymentTransactionRow)]
pub struct PaymentTransaction {
    pub id: PaymentTransactionId,
    pub tenant_id: TenantId,
    // technically we could allow a payment intent to be linked to multiple invoices ? (ex: pay multiple overdue at once)
    pub invoice_id: Option<InvoiceId>,
    pub provider_transaction_id: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub refunded_at: Option<NaiveDateTime>,
    pub amount: i64,
    pub currency: String,
    // TODO fees ?
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    #[map(~.into())]
    pub status: PaymentStatusEnum,
    #[map(~.into())]
    pub payment_type: PaymentTypeEnum,
    // enum ?
    pub error_type: Option<String>,
    pub receipt_pdf_id: Option<StoredDocumentId>,
    pub checkout_session_id: Option<CheckoutSessionId>,
    pub pending_plan_version_id: Option<PlanVersionId>,
    /// Hosted in-flow-capture intent at the provider (Stancer). While the row
    /// is Pending, its presence marks the attempt as resumable: re-initiating
    /// on `pending_connection_id` rehydrates the SAME intent/redirect.
    pub pending_provider_intent_id: Option<String>,
    pub pending_connection_id: Option<CustomerConnectionId>,
    /// Cumulative amount voluntarily refunded on a still-Settled transaction: the Σ of its
    /// settled REFUND children. Leaves the invoice paid — the Refund credit note is what
    /// reduces it.
    pub amount_refunded: i64,
    /// Cumulative amount involuntarily clawed back (chargeback, recall, lost dispute). This is
    /// the one the invoice nets out; a full claw-back flips `status` to Refunded.
    pub amount_reversed: i64,
    #[map(~.map(|k| k.into()))]
    pub reversal_kind: Option<ReversalKind>,
    pub reversal_reason: Option<String>,
    /// The part of `amount_reversed` lost to disputes: the invoice stays paid.
    pub amount_disputed: i64,
    /// Withdrawn by the provider above what the payment still held: never applied.
    pub amount_reversed_excess: i64,
    /// Set on REFUND rows: the settled Payment row the money is given back from.
    pub parent_transaction_id: Option<PaymentTransactionId>,
    /// Set on REFUND rows drawn against a finalized Refund credit note.
    pub credit_note_id: Option<CreditNoteId>,
    #[map(~.map(|m| m.into()))]
    pub refund_mode: Option<RefundMode>,
    #[map(~.map(|k| k.into()))]
    pub decline_kind: Option<DeclineKind>,
    /// Transient — populated only on a just-charged transaction so on-session
    /// callers can surface 3DS without re-fetching. Never read from the DB row.
    #[ghost({None})]
    pub next_action: Option<PaymentNextAction>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaymentIntent {
    pub external_id: String,
    pub transaction_id: PaymentTransactionId,
    pub tenant_id: TenantId,
    pub amount_requested: i64,
    pub amount_received: Option<i64>,
    pub currency: String,
    pub next_action: Option<PaymentNextAction>,
    pub status: PaymentStatusEnum,
    pub last_payment_error: Option<String>,
    pub decline_kind: Option<DeclineKind>,
    pub processed_at: Option<NaiveDateTime>,
}

/// Why a provider's report of a captured payment does not belong to the transaction it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettlementMismatch {
    /// The transaction is bound to another provider payment.
    ProviderPayment { stored: String, reported: String },
    Amount {
        expected_minor: i64,
        expected_currency: String,
        received_minor: i64,
        received_currency: String,
    },
}

impl PaymentTransaction {
    /// Whether `intent` may settle this transaction. A provider that reports no amount
    /// (GoCardless) is matched on its payment id only: the amount is the one we requested.
    pub fn settlement_mismatch(&self, intent: &PaymentIntent) -> Option<SettlementMismatch> {
        if let Some(stored) = &self.provider_transaction_id
            && *stored != intent.external_id
        {
            return Some(SettlementMismatch::ProviderPayment {
                stored: stored.clone(),
                reported: intent.external_id.clone(),
            });
        }
        let received_minor = intent.amount_received?;
        // Some providers report lowercase ISO codes; local rows store uppercase.
        if received_minor != self.amount || !intent.currency.eq_ignore_ascii_case(&self.currency) {
            return Some(SettlementMismatch::Amount {
                expected_minor: self.amount,
                expected_currency: self.currency.clone(),
                received_minor,
                received_currency: intent.currency.clone(),
            });
        }
        None
    }
}

/// A charge on a saved method that waits on the customer's authentication (3DS / SCA), not on the
/// provider. A hosted-redirect attempt also stores a next action on its pre-created row, but it
/// has no method or provider id yet and is resumed by re-initiating the hosted flow instead.
pub fn row_awaits_authentication(row: &PaymentTransactionRow) -> bool {
    row.status == diesel_models::enums::PaymentStatusEnum::Pending
        && row.next_action.is_some()
        && row.pending_provider_intent_id.is_none()
        && row.payment_method_id.is_some()
        && row.provider_transaction_id.is_some()
}

#[derive(Clone, Debug, o2o)]
#[from_owned(PaymentTransactionWithMethodRow)]
pub struct PaymentTransactionWithMethod {
    /// See [`row_awaits_authentication`]. Read before `transaction`, whose conversion drops the
    /// stored action.
    #[from(row_awaits_authentication(&@.transaction))]
    pub awaiting_authentication: bool,
    #[map(~.into())]
    pub transaction: PaymentTransaction,
    #[map(~.map(|m| m.into()))]
    pub method: Option<CustomerPaymentMethod>,
}

#[cfg(test)]
mod tests {
    use super::{
        PaymentIntent, PaymentNextAction, PaymentTransaction, SettlementMismatch,
        row_awaits_authentication,
    };
    use common_domain::ids::{
        BaseId, CustomerConnectionId, CustomerPaymentMethodId, PaymentTransactionId, TenantId,
    };
    use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
    use diesel_models::payments::PaymentTransactionRow;
    use secrecy::SecretString;

    /// A saved-method charge the provider answered with a 3DS challenge.
    fn challenged_row() -> PaymentTransactionRow {
        PaymentTransactionRow {
            id: PaymentTransactionId::new(),
            tenant_id: TenantId::new(),
            invoice_id: None,
            provider_transaction_id: Some("pi_1".into()),
            processed_at: None,
            refunded_at: None,
            amount: 3500,
            currency: "EUR".into(),
            payment_method_id: Some(CustomerPaymentMethodId::new()),
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            receipt_pdf_id: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            store_committed_at: common_clock::now().naive_utc(),
            initiated_by_customer_id: None,
            created_at: common_clock::now(),
            next_action: serde_json::to_value(sdk()).ok(),
            amount_refunded: 0,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            amount_reversed: 0,
            reversal_kind: None,
            reversal_reason: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            last_reconciled_at: None,
            decline_kind: None,
            amount_disputed: 0,
            amount_reversed_excess: 0,
            refund_excess: 0,
            charge_attempt: None,
        }
    }

    #[test]
    fn a_challenged_charge_awaits_authentication_until_it_resolves() {
        assert!(row_awaits_authentication(&challenged_row()));

        let mut settled = challenged_row();
        settled.status = PaymentStatusEnum::Settled;
        assert!(!row_awaits_authentication(&settled));

        let mut in_flight = challenged_row();
        in_flight.next_action = None;
        assert!(!row_awaits_authentication(&in_flight));
    }

    /// An abandoned hosted-redirect attempt also stores a next action, but it is resumed by
    /// re-initiating the hosted flow ("Continue payment"), never offered as a challenge.
    #[test]
    fn a_hosted_redirect_attempt_does_not_await_authentication() {
        let mut hosted = challenged_row();
        hosted.provider_transaction_id = None;
        hosted.payment_method_id = None;
        hosted.pending_provider_intent_id = Some("hosted_intent".into());
        hosted.pending_connection_id = Some(CustomerConnectionId::new());
        hosted.next_action = serde_json::to_value(PaymentNextAction::RedirectToUrl {
            url: "https://provider.example/pay".into(),
        })
        .ok();
        assert!(!row_awaits_authentication(&hosted));
    }

    fn sdk() -> PaymentNextAction {
        PaymentNextAction::UseSdk {
            intent_id: "pi_1".into(),
            publishable_key: "pk_test".into(),
            client_secret: Some(SecretString::from("pi_1_secret_xyz".to_string())),
        }
    }

    #[test]
    fn serialized_form_never_contains_secret() {
        let json = serde_json::to_string(&sdk()).unwrap();
        assert!(
            !json.contains("secret"),
            "serialized form leaked a secret: {json}"
        );
        assert!(json.contains("pk_test") && json.contains("pi_1"));
    }

    #[test]
    fn debug_redacts_secret() {
        let dbg = format!("{:?}", sdk());
        assert!(
            !dbg.contains("pi_1_secret_xyz"),
            "Debug leaked the secret: {dbg}"
        );
    }

    fn success(external_id: &str, received: Option<(i64, &str)>) -> PaymentIntent {
        let tx: PaymentTransaction = challenged_row().into();
        PaymentIntent {
            external_id: external_id.into(),
            transaction_id: tx.id,
            tenant_id: tx.tenant_id,
            amount_requested: received.map_or(0, |(amount, _)| amount),
            amount_received: received.map(|(amount, _)| amount),
            currency: received.map(|(_, c)| c.to_string()).unwrap_or_default(),
            next_action: None,
            status: super::PaymentStatusEnum::Settled,
            last_payment_error: None,
            decline_kind: None,
            processed_at: None,
        }
    }

    #[test]
    fn a_success_with_the_transaction_figures_settles_it() {
        let tx: PaymentTransaction = challenged_row().into();

        assert_eq!(
            tx.settlement_mismatch(&success("pi_1", Some((3500, "EUR")))),
            None
        );
        assert_eq!(
            tx.settlement_mismatch(&success("pi_1", Some((3500, "eur")))),
            None
        );
        assert_eq!(tx.settlement_mismatch(&success("pi_1", None)), None);

        let mut unbound = tx.clone();
        unbound.provider_transaction_id = None;
        assert_eq!(
            unbound.settlement_mismatch(&success("pi_new", Some((3500, "EUR")))),
            None
        );
    }

    #[test]
    fn a_success_with_other_figures_does_not_settle_it() {
        let tx: PaymentTransaction = challenged_row().into();
        let amount = |received_minor: i64, received_currency: &str| {
            Some(SettlementMismatch::Amount {
                expected_minor: 3500,
                expected_currency: "EUR".into(),
                received_minor,
                received_currency: received_currency.into(),
            })
        };

        assert_eq!(
            tx.settlement_mismatch(&success("pi_1", Some((3499, "EUR")))),
            amount(3499, "EUR")
        );
        assert_eq!(
            tx.settlement_mismatch(&success("pi_1", Some((3500, "USD")))),
            amount(3500, "USD")
        );
        assert_eq!(
            tx.settlement_mismatch(&success("pi_other", Some((3500, "EUR")))),
            Some(SettlementMismatch::ProviderPayment {
                stored: "pi_1".into(),
                reported: "pi_other".into(),
            })
        );
        assert!(tx.settlement_mismatch(&success("", None)).is_some());
    }
}
