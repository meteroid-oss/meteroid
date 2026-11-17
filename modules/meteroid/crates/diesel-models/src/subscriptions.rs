use crate::schema::customer;
use crate::schema::plan;
use crate::schema::plan_version;
use chrono::NaiveDate;
use chrono::NaiveDateTime;

use crate::enums::{
    BillingPeriodEnum, SubscriptionActivationConditionEnum, SubscriptionStatusEnum,
};
use common_domain::ids::{
    CustomerId, InvoicingEntityId, PlanId, PlanVersionId, QuoteId, SubscriptionId, TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::subscription)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionRow {
    pub id: SubscriptionId,
    pub customer_id: CustomerId,
    pub billing_day_anchor: i16,
    pub tenant_id: TenantId,
    pub start_date: NaiveDate,
    pub plan_version_id: PlanVersionId,
    pub created_at: NaiveDateTime,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub currency: String,
    pub mrr_cents: i64,
    pub period: BillingPeriodEnum,
    pub pending_checkout: bool,
    pub end_date: Option<NaiveDate>,
    pub trial_duration: Option<i32>,
    pub activation_condition: SubscriptionActivationConditionEnum,
    pub billing_start_date: Option<NaiveDate>,
    pub conn_meta: Option<serde_json::Value>,
    pub status: SubscriptionStatusEnum,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub purchase_order: Option<String>,
    pub quote_id: Option<QuoteId>,
    pub backdate_invoices: bool,
    pub payment_methods_config: Option<serde_json::Value>,
    pub imported_at: Option<NaiveDateTime>,
    pub custom_properties: serde_json::Value,
    pub terminated_at: Option<NaiveDateTime>,
    /// Snapshot of the plan version's setting at creation; see `InvoiceRow::tax_inclusive`.
    pub tax_inclusive: bool,
    /// The tax rate baked into the quoted prices, resolved at creation. `None` when the
    /// subscription is tax-exclusive or bears no tax; see the migration for the semantics.
    pub tax_inclusive_rate: Option<Decimal>,
    pub usage_filter_property: Option<String>,
    pub usage_filter_value: Option<String>,
    pub usage_filter_since: Option<NaiveDateTime>,
    /// Trigger-maintained: true while the tenant is not ACTIVE.
    pub lifecycle_suspended: bool,
    /// The reconcile watermark: periods ending by it were settled before billing from facts.
    pub reconcile_from: Option<NaiveDate>,
    /// When the facts next change the derived state (`lifecycle::derived`).
    pub next_wake_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The watermark of what a period bills at its end, earlier for an import's period.
    pub reconcile_arrears_from: Option<NaiveDate>,
    /// The billing steps failed in a row, never a status: retried forever.
    pub billing_failures: i32,
    pub billing_first_failed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub billing_last_failed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub billing_last_error: Option<String>,
    pub billing_retry_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::subscription)]
pub struct SubscriptionRowNew {
    pub id: SubscriptionId,
    pub customer_id: CustomerId,
    pub billing_day_anchor: i16,
    pub tenant_id: TenantId,
    pub start_date: NaiveDate,
    pub plan_version_id: PlanVersionId,
    pub created_at: NaiveDateTime,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub currency: String,
    pub mrr_cents: i64,
    pub period: BillingPeriodEnum,
    pub pending_checkout: bool,
    pub end_date: Option<NaiveDate>,
    pub trial_duration: Option<i32>,
    pub activation_condition: SubscriptionActivationConditionEnum,
    pub billing_start_date: Option<NaiveDate>,
    pub status: SubscriptionStatusEnum,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub purchase_order: Option<String>,
    pub quote_id: Option<QuoteId>,
    pub backdate_invoices: bool,
    pub payment_methods_config: Option<serde_json::Value>,
    pub imported_at: Option<NaiveDateTime>,
    pub custom_properties: serde_json::Value,
    pub tax_inclusive: bool,
    /// See `SubscriptionRow::tax_inclusive_rate`.
    pub tax_inclusive_rate: Option<Decimal>,
    pub usage_filter_property: Option<String>,
    pub usage_filter_value: Option<String>,
    pub usage_filter_since: Option<NaiveDateTime>,
}

pub struct CancelSubscriptionParams {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub canceled_at: chrono::NaiveDateTime,
    pub billing_end_date: chrono::NaiveDate,
    pub reason: Option<String>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionForDisplayRow {
    #[diesel(embed)]
    pub subscription: SubscriptionRow,
    #[diesel(select_expression = customer::id)]
    #[diesel(select_expression_type = customer::id)]
    pub customer_id: CustomerId,
    #[diesel(select_expression = customer::alias)]
    #[diesel(select_expression_type = customer::alias)]
    pub customer_alias: Option<String>,
    #[diesel(select_expression = customer::name)]
    #[diesel(select_expression_type = customer::name)]
    pub customer_name: String,
    #[diesel(select_expression = customer::invoicing_entity_id)]
    #[diesel(select_expression_type = customer::invoicing_entity_id)]
    pub invoicing_entity_id: InvoicingEntityId,
    #[diesel(select_expression = plan_version::version)]
    #[diesel(select_expression_type = plan_version::version)]
    pub version: i32,
    #[diesel(select_expression = plan::name)]
    #[diesel(select_expression_type = plan::name)]
    pub plan_name: String,
    #[diesel(select_expression = plan::description)]
    #[diesel(select_expression_type = plan::description)]
    pub plan_description: Option<String>,
    #[diesel(select_expression = plan::id)]
    #[diesel(select_expression_type = plan::id)]
    pub plan_id: PlanId,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::subscription)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionCycleRowPatch {
    pub id: SubscriptionId,
    pub tenant_id: TenantId,
    pub status: Option<SubscriptionStatusEnum>,
    pub current_period_start: Option<NaiveDate>,
    pub current_period_end: Option<Option<NaiveDate>>,
    pub pending_checkout: Option<bool>,
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i16>,
    pub terminated_at: Option<Option<NaiveDateTime>>,
}

/// The cadence a subscription renews on, and its current period on that cadence's grid.
#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::subscription)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionCadenceRowPatch {
    pub id: SubscriptionId,
    pub tenant_id: TenantId,
    pub period: BillingPeriodEnum,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
}

#[derive(AsChangeset, Debug, Clone)]
#[diesel(table_name = crate::schema::subscription)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionRowPatch {
    pub charge_automatically: Option<bool>,
    pub auto_advance_invoices: Option<bool>,
    pub net_terms: Option<i32>,
    pub invoice_memo: Option<Option<String>>,
    pub purchase_order: Option<Option<String>>,
    pub payment_methods_config: Option<Option<serde_json::Value>>,
    pub custom_properties: Option<serde_json::Value>,
    pub usage_filter_property: Option<Option<String>>,
    pub usage_filter_value: Option<Option<String>>,
    pub usage_filter_since: Option<Option<NaiveDateTime>>,
}

impl SubscriptionRowPatch {
    pub fn is_empty(&self) -> bool {
        self.charge_automatically.is_none()
            && self.auto_advance_invoices.is_none()
            && self.net_terms.is_none()
            && self.invoice_memo.is_none()
            && self.purchase_order.is_none()
            && self.payment_methods_config.is_none()
            && self.custom_properties.is_none()
            && self.usage_filter_property.is_none()
            && self.usage_filter_value.is_none()
            && self.usage_filter_since.is_none()
    }
}
