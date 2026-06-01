use crate::api_rest::model::{PaginatedRequest, PaginationResponse};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    AddOnId, BillableMetricId, EntitlementId, FeatureId, PlanId, PlanVersionId, ProductId, QuoteId,
    SubscriptionId, string_serde,
};
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

fn default_reset_period() -> ResetPeriod {
    ResetPeriod::Never(NeverResetPeriod {})
}

fn default_metered_enabled_rest() -> bool {
    true
}

/// Lifecycle status of a feature.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::FeatureStatusEnum)]
pub enum FeatureStatus {
    Active,
    /// operator-facing kill switch
    Disabled,
    /// keeps the feature row and its entitlements but hides them from resolution.
    Archived,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::entitlements::PeriodUnit)]
pub enum CalendarUnit {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

/// Resets each time your subscription renews — anchored to your billing cycle.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct BillingCycleResetPeriod {}

/// Resets on calendar boundaries (e.g. the 1st of every month) — not tied to subscription start date.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct CalendarResetPeriod {
    pub unit: CalendarUnit,
    pub interval: u32,
}

/// Resets at regular intervals — anchored to your subscription's exact activation time.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct FixedWindowResetPeriod {
    pub unit: CalendarUnit,
    pub interval: u32,
}

/// Always ends at now — e.g. 30 days means the last 30 days, old usage drops off automatically.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct SlidingWindowResetPeriod {
    pub unit: CalendarUnit,
    pub interval: u32,
}

/// Never resets — counts all usage since the subscription was activated.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct NeverResetPeriod {}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResetPeriod {
    /// Resets each time your subscription renews — anchored to your billing cycle.
    BillingCycle(BillingCycleResetPeriod),
    /// Resets on calendar boundaries (e.g. the 1st of every month) — not tied to subscription start date.
    Calendar(CalendarResetPeriod),
    /// Resets at regular intervals — anchored to your subscription's exact activation time.
    FixedWindow(FixedWindowResetPeriod),
    /// Always ends at now — e.g. 30 days means the last 30 days, old usage drops off automatically.
    SlidingWindow(SlidingWindowResetPeriod),
    /// Never resets — counts all usage since the subscription was activated.
    Never(NeverResetPeriod),
}

impl From<meteroid_store::domain::entitlements::ResetPeriod> for ResetPeriod {
    fn from(v: meteroid_store::domain::entitlements::ResetPeriod) -> Self {
        match v {
            meteroid_store::domain::entitlements::ResetPeriod::BillingCycle => {
                ResetPeriod::BillingCycle(BillingCycleResetPeriod {})
            }
            meteroid_store::domain::entitlements::ResetPeriod::Calendar { unit, interval } => {
                ResetPeriod::Calendar(CalendarResetPeriod {
                    unit: unit.into(),
                    interval,
                })
            }
            meteroid_store::domain::entitlements::ResetPeriod::FixedWindow { unit, interval } => {
                ResetPeriod::FixedWindow(FixedWindowResetPeriod {
                    unit: unit.into(),
                    interval,
                })
            }
            meteroid_store::domain::entitlements::ResetPeriod::SlidingWindow { unit, interval } => {
                ResetPeriod::SlidingWindow(SlidingWindowResetPeriod {
                    unit: unit.into(),
                    interval,
                })
            }
            meteroid_store::domain::entitlements::ResetPeriod::Never => {
                ResetPeriod::Never(NeverResetPeriod {})
            }
        }
    }
}

impl From<ResetPeriod> for meteroid_store::domain::entitlements::ResetPeriod {
    fn from(v: ResetPeriod) -> Self {
        match v {
            ResetPeriod::BillingCycle(_) => {
                meteroid_store::domain::entitlements::ResetPeriod::BillingCycle
            }
            ResetPeriod::Calendar(c) => {
                meteroid_store::domain::entitlements::ResetPeriod::Calendar {
                    unit: c.unit.into(),
                    interval: c.interval,
                }
            }
            ResetPeriod::FixedWindow(f) => {
                meteroid_store::domain::entitlements::ResetPeriod::FixedWindow {
                    unit: f.unit.into(),
                    interval: f.interval,
                }
            }
            ResetPeriod::SlidingWindow(s) => {
                meteroid_store::domain::entitlements::ResetPeriod::SlidingWindow {
                    unit: s.unit.into(),
                    interval: s.interval,
                }
            }
            ResetPeriod::Never(_) => meteroid_store::domain::entitlements::ResetPeriod::Never,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct BooleanFeatureType {}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct MeteredFeatureType {
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FeatureType {
    Boolean(BooleanFeatureType),
    Metered(MeteredFeatureType),
}

impl From<meteroid_store::domain::entitlements::FeatureType> for FeatureType {
    fn from(v: meteroid_store::domain::entitlements::FeatureType) -> Self {
        match v {
            meteroid_store::domain::entitlements::FeatureType::Boolean => {
                FeatureType::Boolean(BooleanFeatureType {})
            }
            meteroid_store::domain::entitlements::FeatureType::Metered { metric_id } => {
                FeatureType::Metered(MeteredFeatureType { metric_id })
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct BooleanEntitlementValue {
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct MeteredEntitlementValue {
    /// Cap on usage. Null means unlimited.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    #[serde(default = "default_reset_period")]
    pub reset_period: ResetPeriod,
    /// Per-entitlement kill switch. `false` means disabled.
    #[serde(default = "default_metered_enabled_rest")]
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntitlementValue {
    Boolean(BooleanEntitlementValue),
    Metered(MeteredEntitlementValue),
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct MeteredEntitlementSpec {
    #[serde(serialize_with = "string_serde::serialize")]
    pub metric_id: BillableMetricId,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    pub reset_period: ResetPeriod,
    pub enabled: bool,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct MeteredEntitlementUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub consumed: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub remaining: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct BooleanEffectiveEntitlementValue {
    pub enabled: bool,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct MeteredEffectiveEntitlementValue {
    pub spec: MeteredEntitlementSpec,
    pub usage: MeteredEntitlementUsage,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EffectiveEntitlementValue {
    Boolean(BooleanEffectiveEntitlementValue),
    Metered(MeteredEffectiveEntitlementValue),
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct BooleanResolvedEntitlementValue {
    pub enabled: bool,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct MeteredResolvedEntitlementValue {
    #[serde(serialize_with = "string_serde::serialize")]
    pub metric_id: BillableMetricId,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    pub reset_period: ResetPeriod,
    pub enabled: bool,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResolvedEntitlementValue {
    Boolean(BooleanResolvedEntitlementValue),
    Metered(MeteredResolvedEntitlementValue),
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct FeatureEntitlementEntity {
    #[serde(with = "common_domain::ids::string_serde")]
    pub id: FeatureId,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct PlanEntitlementEntity {
    #[serde(with = "common_domain::ids::string_serde")]
    pub id: PlanId,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct PlanVersionEntitlementEntity {
    #[serde(with = "common_domain::ids::string_serde")]
    pub id: PlanVersionId,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct AddOnEntitlementEntity {
    #[serde(with = "common_domain::ids::string_serde")]
    pub id: AddOnId,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct SubscriptionEntitlementEntity {
    #[serde(with = "common_domain::ids::string_serde")]
    pub id: SubscriptionId,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct QuoteEntitlementEntity {
    #[serde(with = "common_domain::ids::string_serde")]
    pub id: QuoteId,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntitlementEntity {
    Feature(FeatureEntitlementEntity),
    Plan(PlanEntitlementEntity),
    PlanVersion(PlanVersionEntitlementEntity),
    AddOn(AddOnEntitlementEntity),
    Subscription(SubscriptionEntitlementEntity),
    Quote(QuoteEntitlementEntity),
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct Feature {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: FeatureId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub feature_type: FeatureType,
    pub status: FeatureStatus,
    /// Product this feature belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    pub created_at: DateTime<Utc>,
    /// Feature-level (default) entitlement, applied to all subscriptions as the lowest-priority.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entitlement: Option<Entitlement>,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct FeatureListResponse {
    pub data: Vec<Feature>,
    pub pagination_meta: PaginationResponse,
}

/// A raw entitlement row attached to one entity (feature, plan version, add-on, or subscription).
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct Entitlement {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EntitlementId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub feature_id: FeatureId,
    pub value: EntitlementValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct ProductRef {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: ProductId,
    pub name: String,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct FeatureRef {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: FeatureId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
}

/// Merged entitlement value for a feature for a specific customer, enriched with live usage data.
#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct EffectiveEntitlement {
    pub feature: FeatureRef,
    pub value: EffectiveEntitlementValue,
    /// Highest-priority entity that contributed to the final value, with its human-readable name.
    pub origin: ResolvedOrigin,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct EffectiveEntitlementListResponse {
    pub data: Vec<EffectiveEntitlement>,
}

/// Resolved entity that contributed the winning entitlement value, with a human-readable label.
#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct ResolvedOrigin {
    pub entity: EntitlementEntity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Merged entitlement value for a feature across the priority hierarchy, without usage data.
#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct ResolvedEntitlement {
    pub feature: FeatureRef,
    pub value: ResolvedEntitlementValue,
    /// Highest-priority entity that contributed to the final value, with its human-readable name.
    pub origin: ResolvedOrigin,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct ResolvedEntitlementListResponse {
    pub data: Vec<ResolvedEntitlement>,
}

#[derive(Deserialize, Debug, Clone, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FeatureListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    /// Filter by feature status. Repeat the param to select multiple, omit to return all.
    #[serde(default)]
    pub statuses: Vec<FeatureStatus>,
    /// Filter by product. Omit to return features across all products.
    #[serde(default, with = "common_domain::ids::string_serde_opt")]
    pub product_id: Option<ProductId>,
    /// Search by feature name.
    pub search: Option<String>,
}
