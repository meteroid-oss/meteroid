use chrono::{NaiveDate, NaiveDateTime};
use o2o::o2o;
use uuid::Uuid;

use crate::domain::{
    BillableMetric, CreateSubscriptionComponents, Schedule, SubscriptionComponent,
};
use diesel_models::subscriptions::Subscription as DieselSubscription;
use diesel_models::subscriptions::SubscriptionNew as DieselSubscriptionNew;

#[derive(Debug, o2o)]
#[from_owned(DieselSubscription)]
pub struct CreatedSubscription {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub billing_day: i16,
    pub tenant_id: Uuid,
    pub currency: String,
    pub trial_start_date: Option<NaiveDate>,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub plan_version_id: Uuid,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub cancellation_reason: Option<String>,
    pub mrr_cents: i64,
}

#[derive(Debug)]
pub struct Subscription {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub billing_day: i16,
    pub tenant_id: Uuid,
    pub currency: String,
    pub trial_start_date: Option<NaiveDate>,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub plan_id: Uuid,
    pub plan_name: String,
    pub plan_version_id: Uuid,
    pub version: u32,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    // pub created_by_name: String,
    pub net_terms: u32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub cancellation_reason: Option<String>,
    pub mrr_cents: u64,
}

impl Into<Subscription> for diesel_models::subscriptions::SubscriptionForDisplay {
    fn into(self) -> Subscription {
        Subscription {
            id: self.subscription.id,
            customer_id: self.subscription.customer_id,
            customer_name: self.customer_name,
            customer_alias: self.customer_external_id,
            billing_day: self.subscription.billing_day,
            tenant_id: self.subscription.tenant_id,
            currency: self.subscription.currency,
            trial_start_date: self.subscription.trial_start_date,
            billing_start_date: self.subscription.billing_start_date,
            billing_end_date: self.subscription.billing_end_date,
            plan_id: self.plan_id,
            plan_name: self.plan_name,
            plan_version_id: self.subscription.plan_version_id,
            version: self.version as u32,
            created_at: self.subscription.created_at,
            created_by: self.subscription.created_by,
            net_terms: self.subscription.net_terms as u32,
            invoice_memo: self.subscription.invoice_memo,
            invoice_threshold: self.subscription.invoice_threshold,
            activated_at: self.subscription.activated_at,
            canceled_at: self.subscription.canceled_at,
            cancellation_reason: self.subscription.cancellation_reason,
            mrr_cents: self.subscription.mrr_cents as u64,
        }
    }
}

#[derive(Debug, Clone, o2o)]
#[owned_into(DieselSubscriptionNew)]
#[ghosts(id: {uuid::Uuid::now_v7()}, mrr_cents: {0})]
pub struct SubscriptionNew {
    pub customer_id: Uuid,
    pub billing_day: i16,
    pub tenant_id: Uuid,
    pub currency: String,
    pub trial_start_date: Option<NaiveDate>,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub plan_version_id: Uuid,
    pub created_by: Uuid,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub activated_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone)]
pub struct CreateSubscription {
    pub subscription: SubscriptionNew,
    pub price_components: Option<CreateSubscriptionComponents>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionDetails {
    pub id: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub customer_id: uuid::Uuid,
    pub plan_version_id: uuid::Uuid,
    pub customer_external_id: Option<String>,
    pub billing_start_date: chrono::NaiveDate,
    pub billing_end_date: Option<chrono::NaiveDate>,
    pub billing_day: i16,

    pub currency: String,
    pub net_terms: u32,
    pub schedules: Vec<Schedule>,
    pub price_components: Vec<SubscriptionComponent>,
    pub metrics: Vec<BillableMetric>,
    pub mrr_cents: u64,

    //
    pub version: u32,
    pub plan_name: String,
    pub plan_id: Uuid,
    pub customer_name: String,
    pub canceled_at: Option<chrono::NaiveDateTime>,

    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub created_at: chrono::NaiveDateTime,
    pub cancellation_reason: Option<String>,
    pub activated_at: Option<chrono::NaiveDateTime>,
    pub created_by: Uuid,
    pub trial_start_date: Option<chrono::NaiveDate>,
}
