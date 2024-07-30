use chrono::{NaiveDate, NaiveDateTime};

use o2o::o2o;
use uuid::Uuid;

use crate::domain::enums::BillingPeriodEnum;
use crate::domain::{
    BillableMetric, CreateSubscriptionComponents, Schedule, SubscriptionComponent,
};
use diesel_models::subscriptions::SubscriptionRowNew;
use diesel_models::subscriptions::{
    SubscriptionForDisplayRow, SubscriptionInvoiceCandidateRow, SubscriptionRow,
};

#[derive(Debug, Clone, o2o)]
#[from_owned(SubscriptionRow)]
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
    #[from(~.into())]
    pub period: BillingPeriodEnum,
}

#[derive(Debug, Clone)]
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
    pub period: BillingPeriodEnum,
}

impl Into<Subscription> for SubscriptionForDisplayRow {
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
            period: self.subscription.period.into(),
        }
    }
}

#[derive(Debug, Clone)]
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

impl SubscriptionNew {
    pub fn map_to_row(
        self,
        period: BillingPeriodEnum,
        should_activate: bool,
    ) -> SubscriptionRowNew {
        SubscriptionRowNew {
            id: uuid::Uuid::now_v7(),
            customer_id: self.customer_id,
            billing_day: self.billing_day,
            tenant_id: self.tenant_id,
            currency: self.currency,
            trial_start_date: self.trial_start_date,
            billing_start_date: self.billing_start_date,
            billing_end_date: self.billing_end_date,
            plan_version_id: self.plan_version_id,
            created_by: self.created_by,
            net_terms: self.net_terms,
            invoice_memo: self.invoice_memo,
            invoice_threshold: self.invoice_threshold,
            activated_at: if should_activate {
                Some(chrono::Utc::now().naive_utc())
            } else {
                self.activated_at
            },
            mrr_cents: 0,
            period: period.into(),
        }
    }
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
    pub period: BillingPeriodEnum,
}

#[derive(Debug, Clone)]
pub struct SubscriptionInvoiceCandidate {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub plan_version_id: Uuid,
    pub plan_name: String,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub billing_day: i16,
    pub activated_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    // pub plan_id: Uuid,
    pub currency: String,
    pub net_terms: i32,
    // pub version: i32,
    pub period: BillingPeriodEnum,
}

impl Into<SubscriptionInvoiceCandidate> for SubscriptionInvoiceCandidateRow {
    fn into(self) -> SubscriptionInvoiceCandidate {
        SubscriptionInvoiceCandidate {
            id: self.subscription.id,
            tenant_id: self.subscription.tenant_id,
            customer_id: self.subscription.customer_id,
            plan_version_id: self.subscription.plan_version_id,
            billing_start_date: self.subscription.billing_start_date,
            billing_end_date: self.subscription.billing_end_date,
            billing_day: self.subscription.billing_day,
            activated_at: self.subscription.activated_at,
            canceled_at: self.subscription.canceled_at,
            // plan_id: self.plan_version.plan_id,
            plan_name: self.plan_version.plan_name,
            currency: self.plan_version.currency,
            net_terms: self.plan_version.net_terms,
            // version: self.plan_version.version,
            period: self.subscription.period.into(),
        }
    }
}
