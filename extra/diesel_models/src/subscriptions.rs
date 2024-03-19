use chrono::NaiveDate;
use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::BillingPeriodEnum;
use bigdecimal::BigDecimal;
use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::subscription)]
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

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::subscription)]
pub struct SubscriptionNew {
    pub id: Uuid,
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
