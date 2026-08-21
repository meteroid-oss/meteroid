use crate::domain::PaymentMethodTypeEnum;
use chrono::{DateTime, Utc};

/// Provider-agnostic webhook event the core reacts to. Adapters parse their
/// wire format and emit one of these (or `None` for ignored events).
#[derive(Debug, Clone)]
pub struct NormalizedWebhookEvent {
    pub provider_event_id: String,
    pub provider_event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub kind: NormalizedEventKind,
    /// base62 `TenantId` from the event's `meteroid.tenant_id` metadata, when the
    /// provider echoes it. Lets the dispatcher drop an event delivered to the
    /// wrong tenant's endpoint (one provider account shared by two tenants).
    pub owner_tenant_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum NormalizedEventKind {
    // ── payment lifecycle ─────────────────────────────────────────────
    PaymentSucceeded(PaymentSucceededEvent),
    PaymentFailed(PaymentFailedEvent),
    PaymentPending(PaymentPendingEvent),
    PaymentRequiresAction(PaymentRequiresActionEvent),
    PaymentRefunded(PaymentRefundedEvent),
    /// A settled payment fully clawed back by the bank (GoCardless
    /// `charged_back` / `late_failure_settled`). Full-amount, no partial.
    PaymentReversed(PaymentReversedEvent),
    /// Inverse of [`Self::PaymentReversed`]: the bank cancelled the chargeback
    /// and returned the funds (GoCardless `chargeback_cancelled`). Full-amount.
    PaymentReinstated(PaymentReinstatedEvent),
    /// A provider-side refund observed via webhook that carries no amounts on
    /// the wire (GoCardless `refunds.*` events only link the refund id). The
    /// handler resolves the parent payment and its cumulative refunded total
    /// through the connector (`RefundOps::fetch_refund`).
    RefundObserved {
        external_refund_id: String,
    },

    // ── mandate / payment-method lifecycle ────────────────────────────
    /// Stripe: `setup_intent.succeeded`. GoCardless: `mandates.active`.
    PaymentMethodAttached(PaymentMethodAttachedEvent),
    /// A hosted mandate-setup intent finished and must be finalized by fetching
    /// it from the provider (GoCardless `billing_requests.fulfilled`): the
    /// handler calls `complete_mandate_setup(provider_intent_id)` to recover the
    /// mandate + our metadata, attach the method, and charge any named invoice.
    /// `provider_intent_id` is the GoCardless Billing Request id.
    MandateSetupCompleted {
        provider_intent_id: String,
    },
    /// Provider auto-updated card details (network update / replacement).
    PaymentMethodUpdated(PaymentMethodUpdatedEvent),
    PaymentMethodDetached(PaymentMethodDetachedEvent),
    PaymentMethodExpiring(PaymentMethodExpiringEvent),

    // ── disputes / chargebacks ────────────────────────────────────────
    DisputeOpened(DisputeEvent),
    DisputeFundsWithdrawn(DisputeEvent),
    DisputeWon(DisputeEvent),
    DisputeLost(DisputeEvent),
    DisputeFundsReinstated(DisputeEvent),

    /// Recognized but not acted on; kept so we can log it for forensics.
    Acknowledged {
        reason: &'static str,
    },
}

/// Coarse kinds for configuring webhook subscriptions; the adapter maps each to
/// the concrete provider event names it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedEventSubscription {
    Payments,
    Mandates,
    Refunds,
    Disputes,
}

// ── event payloads ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PaymentSucceededEvent {
    pub external_transaction_id: String,
    pub amount_received_minor: i64,
    pub currency: String,
    pub meteroid_transaction_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaymentFailedEvent {
    pub external_transaction_id: String,
    pub code: Option<String>,
    pub message: String,
    pub retryable: bool,
    pub meteroid_transaction_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaymentPendingEvent {
    pub external_transaction_id: String,
    pub meteroid_transaction_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaymentRequiresActionEvent {
    pub external_transaction_id: String,
    pub action_url: Option<String>,
    pub client_secret: Option<String>,
    pub meteroid_transaction_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaymentRefundedEvent {
    pub external_transaction_id: String,
    pub external_refund_id: String,
    pub amount_refunded_minor: i64,
    pub currency: String,
}

/// A settled payment fully reversed by the bank. Full-amount by nature (GC
/// chargebacks/late failures reclaim the whole payment); no partial figure.
#[derive(Debug, Clone)]
pub struct PaymentReversedEvent {
    pub external_transaction_id: String,
    pub meteroid_transaction_id: Option<String>,
    pub reason: String,
}

/// Inverse of [`PaymentReversedEvent`]: the previously clawed-back funds were
/// handed back to the merchant. Full-amount by nature.
#[derive(Debug, Clone)]
pub struct PaymentReinstatedEvent {
    pub external_transaction_id: String,
    pub meteroid_transaction_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct PaymentMethodAttachedEvent {
    pub external_customer_id: String,
    pub external_payment_method_id: String,
    pub payment_method_type: PaymentMethodTypeEnum,
    pub meteroid_connection_id: Option<String>,
    pub meteroid_customer_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaymentMethodUpdatedEvent {
    pub external_payment_method_id: String,
    pub card_brand: Option<String>,
    pub card_last4: Option<String>,
    pub card_exp_month: Option<i32>,
    pub card_exp_year: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct PaymentMethodDetachedEvent {
    pub external_payment_method_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaymentMethodExpiringEvent {
    pub external_payment_method_id: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DisputeEvent {
    pub external_dispute_id: String,
    pub external_transaction_id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub reason: Option<String>,
}
