use crate::domain::PaymentMethodTypeEnum;
use chrono::{DateTime, Utc};

/// Provider-agnostic webhook event the core code reacts to.
///
/// Each adapter parses its provider's wire format and emits one of these (or
/// `None` for events the core ignores). The core never sees a Stripe /
/// GoCardless / Adyen type.
#[derive(Debug, Clone)]
pub struct NormalizedWebhookEvent {
    pub provider_event_id: String,
    pub provider_event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub kind: NormalizedEventKind,
}

#[derive(Debug, Clone)]
pub enum NormalizedEventKind {
    // ── payment lifecycle ─────────────────────────────────────────────
    PaymentSucceeded(PaymentSucceededEvent),
    PaymentFailed(PaymentFailedEvent),
    PaymentPending(PaymentPendingEvent),
    PaymentRequiresAction(PaymentRequiresActionEvent),
    PaymentRefunded(PaymentRefundedEvent),

    // ── mandate / payment-method lifecycle ────────────────────────────
    /// A new payment method became available for off-session use.
    /// Stripe: `setup_intent.succeeded`. GoCardless: `mandates.active`.
    PaymentMethodAttached(PaymentMethodAttachedEvent),
    /// Provider auto-updated card details (network update / card expiring replacement).
    PaymentMethodUpdated(PaymentMethodUpdatedEvent),
    /// Mandate / payment method is no longer usable.
    PaymentMethodDetached(PaymentMethodDetachedEvent),
    /// Card / mandate is approaching expiry — surface a renewal prompt.
    PaymentMethodExpiring(PaymentMethodExpiringEvent),

    // ── disputes / chargebacks ────────────────────────────────────────
    DisputeOpened(DisputeEvent),
    DisputeFundsWithdrawn(DisputeEvent),
    DisputeWon(DisputeEvent),
    DisputeLost(DisputeEvent),
    DisputeFundsReinstated(DisputeEvent),

    /// Event the adapter recognized but the core does not need to act on.
    /// Kept so we can log / store it for forensics without ignoring silently.
    Acknowledged { reason: &'static str },
}

/// Coarse-grained kind identifiers used when configuring webhook subscriptions
/// (e.g. when self-registering an endpoint with the provider). The adapter
/// maps each of these to the concrete provider event names it needs.
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
