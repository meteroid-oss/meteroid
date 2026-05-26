use super::enums::{PaymentStatusEnum, PaymentTypeEnum};
use chrono::NaiveDateTime;

use crate::domain::CustomerPaymentMethod;
use common_domain::ids::{
    CheckoutSessionId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, PlanVersionId,
    StoredDocumentId, TenantId,
};
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionWithMethodRow};
use o2o::o2o;
use serde::{Deserialize, Serialize};

/// Customer action required to complete a charge (3DS / SCA). Stored in
/// `payment_transaction.next_action` (JSONB) and surfaced to the portal.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaymentNextAction {
    /// Redirect the browser to this provider-hosted URL (3DS redirect, bank app).
    RedirectToUrl { url: String },
    /// Client SDK completes the action with the intent's client secret
    /// (Stripe.js `handleNextAction`).
    UseSdk {
        intent_id: String,
        publishable_key: String,
        /// Sensitive: lets the holder complete this PaymentIntent. Transient
        /// only — returned to the portal in the charge response but NEVER
        /// persisted (stripped via [`Self::for_storage`]); resumed flows
        /// re-fetch it from the provider.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
    },
}

impl PaymentNextAction {
    /// Storage projection — drops the transient client secret so it never
    /// lands in the database.
    pub fn for_storage(&self) -> Self {
        match self {
            Self::RedirectToUrl { url } => Self::RedirectToUrl { url: url.clone() },
            Self::UseSdk {
                intent_id,
                publishable_key,
                ..
            } => Self::UseSdk {
                intent_id: intent_id.clone(),
                publishable_key: publishable_key.clone(),
                client_secret: None,
            },
        }
    }
}

// Manual Debug so the client secret can never leak into logs.
impl std::fmt::Debug for PaymentNextAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RedirectToUrl { url } => {
                f.debug_struct("RedirectToUrl").field("url", url).finish()
            }
            Self::UseSdk {
                intent_id,
                publishable_key,
                client_secret,
            } => f
                .debug_struct("UseSdk")
                .field("intent_id", intent_id)
                .field("publishable_key", publishable_key)
                .field(
                    "client_secret",
                    &client_secret.as_ref().map(|_| "<redacted>"),
                )
                .finish(),
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
