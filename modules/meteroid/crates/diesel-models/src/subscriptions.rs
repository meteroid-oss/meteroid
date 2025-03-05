use crate::schema::customer;
use crate::schema::plan;
use crate::schema::plan_version;
use chrono::NaiveDate;
use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::{BillingPeriodEnum, PaymentMethodTypeEnum, SubscriptionActivationConditionEnum};
use common_domain::ids::{
    CustomerConnectionId, CustomerId, CustomerPaymentMethodId, PlanId, SubscriptionId, TenantId,
};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
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
    pub plan_version_id: Uuid,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub cancellation_reason: Option<String>,
    pub mrr_cents: i64,
    pub period: BillingPeriodEnum,
    pub currency: String,
    pub psp_connection_id: Option<CustomerConnectionId>,
    pub pending_checkout: bool,
    // this is used if payment_method is null (ex: payment method deleted) to elect a new payment method/start a checkout
    pub payment_method_type: Option<PaymentMethodTypeEnum>,
    pub payment_method: Option<CustomerPaymentMethodId>,
    pub end_date: Option<NaiveDate>,
    pub trial_duration: Option<i32>,
    pub activation_condition: SubscriptionActivationConditionEnum,
    pub billing_start_date: Option<NaiveDate>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::subscription)]
pub struct SubscriptionRowNew {
    pub id: SubscriptionId,
    pub customer_id: CustomerId,
    pub billing_day_anchor: i16,
    pub tenant_id: TenantId,
    pub start_date: NaiveDate,
    pub plan_version_id: Uuid,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub currency: String,
    pub psp_connection_id: Option<CustomerConnectionId>,
    pub mrr_cents: i64,
    pub period: BillingPeriodEnum,
    pub pending_checkout: bool,
    pub payment_method: Option<CustomerPaymentMethodId>,
    pub end_date: Option<NaiveDate>,
    pub trial_duration: Option<i32>,
    pub activation_condition: SubscriptionActivationConditionEnum,
    pub billing_start_date: Option<NaiveDate>,
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

    use common_domain::ids::{CustomerId, PlanId, SubscriptionId, TenantId};
    use diesel::{Queryable, Selectable};
    use uuid::Uuid;

    #[derive(Debug, Queryable, Selectable)]
    #[diesel(table_name = crate::schema::subscription)]
    #[diesel(check_for_backend(diesel::pg::Pg))]
    pub struct SubscriptionEmbedRow {
        pub id: SubscriptionId,
        pub tenant_id: TenantId,
        pub customer_id: CustomerId,
        pub plan_version_id: Uuid,
        pub start_date: NaiveDate,
        pub end_date: Option<NaiveDate>,
        pub billing_start_date: Option<NaiveDate>,
        pub billing_day_anchor: i16,
        pub net_terms: i32,
        pub activated_at: Option<NaiveDateTime>,
        pub canceled_at: Option<NaiveDateTime>,
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
