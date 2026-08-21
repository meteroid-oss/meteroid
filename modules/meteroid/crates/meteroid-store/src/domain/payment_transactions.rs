use super::enums::{PaymentStatusEnum, PaymentTypeEnum};
use chrono::NaiveDateTime;

use crate::domain::CustomerPaymentMethod;
use common_domain::ids::{
    CheckoutSessionId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, PlanVersionId,
    StoredDocumentId, TenantId,
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
    /// Cumulative amount clawed back on a still-Settled transaction (partial
    /// refunds); a full claw-back flips `status` to Refunded instead.
    pub amount_refunded: i64,
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
    pub processed_at: Option<NaiveDateTime>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(PaymentTransactionWithMethodRow)]
pub struct PaymentTransactionWithMethod {
    #[map(~.into())]
    pub transaction: PaymentTransaction,
    #[map(~.map(|m| m.into()))]
    pub method: Option<CustomerPaymentMethod>,
}

#[cfg(test)]
mod tests {
    use super::PaymentNextAction;
    use secrecy::SecretString;

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
}
