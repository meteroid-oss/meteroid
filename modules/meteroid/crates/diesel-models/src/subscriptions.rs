use crate::schema::customer;
use crate::schema::plan;
use crate::schema::plan_version;
use chrono::NaiveDate;
use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::{
    BillingPeriodEnum, CycleActionEnum, SubscriptionActivationConditionEnum, SubscriptionStatusEnum,
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
    pub created_by: Uuid,
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
    pub cycle_index: Option<i32>,
    pub status: SubscriptionStatusEnum,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
    pub next_cycle_action: Option<CycleActionEnum>,
    pub last_error: Option<String>,
    pub error_count: i32,
    pub next_retry: Option<NaiveDateTime>,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub purchase_order: Option<String>,
    pub quote_id: Option<QuoteId>,
    pub backdate_invoices: bool,
    pub processing_started_at: Option<NaiveDateTime>,
    pub payment_methods_config: Option<serde_json::Value>,
    pub imported_at: Option<NaiveDateTime>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::subscription)]
pub struct SubscriptionRowNew {
    pub id: SubscriptionId,
    pub customer_id: CustomerId,
    pub billing_day_anchor: i16,
    pub tenant_id: TenantId,
    pub start_date: NaiveDate,
    pub plan_version_id: PlanVersionId,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
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
    pub cycle_index: Option<i32>,
    pub status: SubscriptionStatusEnum,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
    pub next_cycle_action: Option<CycleActionEnum>,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub purchase_order: Option<String>,
    pub quote_id: Option<QuoteId>,
    pub backdate_invoices: bool,
    pub payment_methods_config: Option<serde_json::Value>,
    pub imported_at: Option<NaiveDateTime>,
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
    pub cycle_index: Option<i32>,
    pub status: Option<SubscriptionStatusEnum>,
    pub next_cycle_action: Option<Option<CycleActionEnum>>,
    pub current_period_start: Option<NaiveDate>,
    pub current_period_end: Option<Option<NaiveDate>>,
    pub pending_checkout: Option<bool>,
    pub processing_started_at: Option<Option<NaiveDateTime>>,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::subscription)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionCycleErrorRowPatch {
    pub id: SubscriptionId,
    pub tenant_id: TenantId,
    pub last_error: Option<Option<String>>,
    pub next_retry: Option<Option<NaiveDateTime>>,
    pub error_count: Option<i32>,
    pub status: Option<SubscriptionStatusEnum>,
}

#[derive(AsChangeset, Debug)]
#[diesel(table_name = crate::schema::subscription)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionRowPatch {
    pub charge_automatically: Option<bool>,
    pub auto_advance_invoices: Option<bool>,
    pub net_terms: Option<i32>,
    pub invoice_memo: Option<Option<String>>,
    pub purchase_order: Option<Option<String>>,
    pub payment_methods_config: Option<Option<serde_json::Value>>,
}
