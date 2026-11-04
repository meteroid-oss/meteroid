use crate::subscriptions::model::SubscriptionStatusEnum;
use chrono::NaiveDate;
use common_domain::ids::{CustomerId, PlanId, SubscriptionId, string_serde};
use meteroid_rest_common::model::BillingPeriodEnum;
use serde::Serialize;
use utoipa::ToSchema;

/// The customer the token was minted for.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ClientCustomer {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: CustomerId,
    pub alias: Option<String>,
    pub name: String,
    pub currency: String,
    pub billing_email: Option<String>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ClientSubscription {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: SubscriptionId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub plan_id: PlanId,
    pub plan_name: String,
    pub plan_version: u32,
    pub status: SubscriptionStatusEnum,
    pub currency: String,
    /// Billing period (monthly, annual, etc.)
    pub billing_period: Option<BillingPeriodEnum>,
    /// When the subscription contract starts (benefits apply from this date)
    pub start_date: NaiveDate,
    /// When the subscription ends (if set)
    pub end_date: Option<NaiveDate>,
    /// Current billing period start date
    pub current_period_start: NaiveDate,
    /// Current billing period end date
    pub current_period_end: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ClientSubscriptionListResponse {
    pub data: Vec<ClientSubscription>,
}
