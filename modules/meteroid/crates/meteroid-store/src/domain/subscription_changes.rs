use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::subscription_components::SubscriptionFee;
use chrono::NaiveDate;
use common_domain::ids::{CheckoutSessionId, InvoiceId, PriceComponentId, ProductId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanChangeMode {
    Immediate,
    EndOfPeriod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeDirection {
    Upgrade,
    Downgrade,
    Lateral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanChangePreview {
    pub matched: Vec<MatchedComponent>,
    pub added: Vec<AddedComponent>,
    pub removed: Vec<RemovedComponent>,
    pub effective_date: NaiveDate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanChangePreviewExtended {
    pub preview: PlanChangePreview,
    pub proration: Option<ProrationSummary>,
    pub change_direction: ChangeDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProrationSummary {
    pub credits_total_cents: i64,
    pub charges_total_cents: i64,
    pub net_amount_cents: i64,
    pub proration_factor: f64,
    pub days_remaining: u32,
    pub days_in_period: u32,
}

#[derive(Debug, Clone)]
pub struct ImmediatePlanChangeResult {
    pub adjustment_invoice_id: Option<InvoiceId>,
    pub effective_date: NaiveDate,
}

/// Result of an immediate plan change that requires payment.
#[derive(Debug, Clone)]
pub enum PlanChangePaymentResult {
    /// Payment settled immediately — plan change applied.
    Completed(ImmediatePlanChangeResult),
    /// Payment is pending (processing, 3DS, etc.) — plan change deferred until settlement.
    AwaitingPayment {
        adjustment_invoice_id: InvoiceId,
        transaction: PaymentTransaction,
        effective_date: NaiveDate,
    },
    /// Payment failed or no saved card — checkout session created for user to complete payment.
    CheckoutRequired {
        checkout_session_id: CheckoutSessionId,
        checkout_token: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedComponent {
    pub product_id: ProductId,
    pub current_name: String,
    pub current_fee: SubscriptionFee,
    pub current_period: SubscriptionFeeBillingPeriod,
    pub new_name: String,
    pub new_fee: SubscriptionFee,
    pub new_period: SubscriptionFeeBillingPeriod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddedComponent {
    pub name: String,
    pub fee: SubscriptionFee,
    pub period: SubscriptionFeeBillingPeriod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovedComponent {
    pub name: String,
    pub current_fee: SubscriptionFee,
    pub current_period: SubscriptionFeeBillingPeriod,
}

/// Proration result for a plan change — contains individual line items.
#[derive(Debug, Clone)]
pub struct ProrationResult {
    pub lines: Vec<ProrationLineItem>,
    pub net_amount_cents: i64,
    pub change_date: NaiveDate,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub proration_factor: f64,
}

#[derive(Debug, Clone)]
pub struct ProrationLineItem {
    pub name: String,
    pub amount_cents: i64,
    pub full_period_amount_cents: i64,
    pub is_credit: bool,
    pub product_id: Option<ProductId>,
    pub price_component_id: Option<PriceComponentId>,
}
