use crate::api_rest::currencies::model::Currency;
use crate::api_rest::model::PaginatedRequest;
use chrono::NaiveDate;
use common_domain::ids::{CustomerId, PlanVersionId};
use common_domain::ids::{PlanId, string_serde_opt};
use common_domain::ids::{SubscriptionId, string_serde};
use utoipa::ToSchema;
use validator::Validate;

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct SubscriptionRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(default, with = "string_serde_opt")]
    pub customer_id: Option<CustomerId>,
    #[serde(default, with = "string_serde_opt")]
    pub plan_id: Option<PlanId>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Subscription {
    #[serde(with = "string_serde")]
    pub id: SubscriptionId,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub billing_day_anchor: i16,
    pub currency: Currency,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionDetails {
    #[serde(with = "string_serde")]
    pub id: SubscriptionId,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub billing_day_anchor: i16,
    pub currency: Currency,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate, Debug)]
pub struct SubscriptionCreateRequest {
    #[serde(with = "string_serde")]
    pub plan_version_id: PlanVersionId,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    pub trial_days: Option<u32>,
    #[schema(example = "2024-11-01")]
    pub start_date: NaiveDate,
    #[schema(example = "2025-11-01")]
    pub end_date: Option<NaiveDate>,
    #[validate(range(min = 1, max = 32767))]
    #[schema(minimum = 1, maximum = 32767)]
    pub billing_day_anchor: Option<u16>,
    pub net_terms: Option<u32>,
    pub invoice_memo: Option<String>,
    #[schema(example = "19.99", format = "decimal")]
    pub invoice_threshold: Option<String>,

    pub activation_condition: SubscriptionActivationCondition,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub enum SubscriptionActivationCondition {
    #[serde(rename = "ON_START")]
    OnStart,
    #[serde(rename = "ON_CHECKOUT")]
    OnCheckout,
    #[serde(rename = "MANUAL")]
    Manual,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub enum SubscriptionFeeBillingPeriod {
    #[serde(rename = "ONETIME")]
    OneTime,
    #[serde(rename = "MONTHLY")]
    Monthly,
    #[serde(rename = "QUARTERLY")]
    Quarterly,
    #[serde(rename = "ANNUAL")]
    Annual,
}
