use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use diesel_models::enums::BillingPeriodEnum;
use o2o::o2o;
use uuid::Uuid;

use diesel_models::subscriptions::Subscription as DieselSubscription;
use diesel_models::subscriptions::SubscriptionNew as DieselSubscriptionNew;

#[derive(Debug, o2o)]
#[from_owned(DieselSubscription)]
#[owned_into(DieselSubscription)]
pub struct Subscription {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub billing_day: i16,
    pub tenant_id: Uuid,
    pub trial_start_date: Option<NaiveDate>,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub plan_version_id: Uuid,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub input_parameters: Option<serde_json::Value>,
    pub effective_billing_period: BillingPeriodEnum,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<BigDecimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub cancellation_reason: Option<String>,
}

#[derive(Debug, o2o)]
#[owned_into(DieselSubscriptionNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct SubscriptionNew {
    pub customer_id: Uuid,
    pub billing_day: i16,
    pub tenant_id: Uuid,
    pub trial_start_date: Option<NaiveDate>,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub plan_version_id: Uuid,
    pub created_by: Uuid,
    pub input_parameters: Option<serde_json::Value>,
    pub effective_billing_period: BillingPeriodEnum,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<BigDecimal>,
    pub activated_at: Option<NaiveDateTime>,
}
