use crate::schema::customer;
use crate::schema::plan;
use crate::schema::plan_version;
use chrono::NaiveDate;
use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::{
    BillingPeriodEnum, CycleActionEnum, PaymentMethodTypeEnum, SubscriptionActivationConditionEnum,
    SubscriptionStatusEnum,
};
use common_domain::ids::{
    BankAccountId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, InvoicingEntityId,
    PlanId, PlanVersionId, SubscriptionId, TenantId,
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
    pub card_connection_id: Option<CustomerConnectionId>,
    pub direct_debit_connection_id: Option<CustomerConnectionId>,
    pub bank_account_id: Option<BankAccountId>,
    pub pending_checkout: bool,
    // this is used if payment_method is null (ex: payment method deleted) to elect a new payment method/start a checkout
    pub payment_method_type: Option<PaymentMethodTypeEnum>,
    pub payment_method: Option<CustomerPaymentMethodId>,
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
    pub card_connection_id: Option<CustomerConnectionId>,
    pub direct_debit_connection_id: Option<CustomerConnectionId>,
    pub bank_account_id: Option<BankAccountId>,
    pub mrr_cents: i64,
    pub period: BillingPeriodEnum,
    pub pending_checkout: bool,
    pub payment_method: Option<CustomerPaymentMethodId>,
    // TODO payment_method_type
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
    #[diesel(select_expression = plan::id)]
    #[diesel(select_expression_type = plan::id)]
    pub plan_id: PlanId,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionInvoiceCandidateRow {
    #[diesel(embed)]
    pub subscription: subscription_invoice_candidate::SubscriptionEmbedRow,
    #[diesel(embed)]
    pub plan_version: subscription_invoice_candidate::PlanVersionEmbedRow,
}

mod subscription_invoice_candidate {
    use crate::enums::BillingPeriodEnum;

    use chrono::{NaiveDate, NaiveDateTime};

    use common_domain::ids::{CustomerId, PlanId, PlanVersionId, SubscriptionId, TenantId};
    use diesel::{Queryable, Selectable};

    #[derive(Debug, Queryable, Selectable)]
    #[diesel(table_name = crate::schema::subscription)]
    #[diesel(check_for_backend(diesel::pg::Pg))]
    pub struct SubscriptionEmbedRow {
        pub id: SubscriptionId,
        pub tenant_id: TenantId,
        pub customer_id: CustomerId,
        pub plan_version_id: PlanVersionId,
        pub start_date: NaiveDate,
        pub end_date: Option<NaiveDate>,
        pub billing_start_date: Option<NaiveDate>,
        pub billing_day_anchor: i16,
        pub net_terms: i32,
        pub activated_at: Option<NaiveDateTime>,
        pub period: BillingPeriodEnum,
    }

    #[derive(Debug, Queryable, Selectable)]
    #[diesel(table_name = crate::schema::plan_version)]
    #[diesel(check_for_backend(diesel::pg::Pg))]
    pub struct PlanVersionEmbedRow {
        pub plan_id: PlanId,
        pub currency: String,
        pub net_terms: i32,
        pub version: i32,
        #[diesel(select_expression = crate::schema::plan::name)]
        #[diesel(select_expression_type = crate::schema::plan::name)]
        pub plan_name: String,
    }
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
}
