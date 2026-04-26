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
    ResetPeriod::Never
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

/// What happens past the limit.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::entitlements::OverageBehavior)]
pub enum OverageBehavior {
    /// Deny access once usage reaches the limit. Meteroid does not enforce this — your integration must check and act.
    Block { grace_period_pct: Option<u32> },
    /// Keep serving usage past the limit; overage is billed or handled out-of-band.
    Allow,
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

#[derive(o2o, Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::entitlements::ResetPeriod)]
pub enum ResetPeriod {
    /// Resets at the start of each subscription billing period.
    BillingCycle,
    /// Buckets aligned to the calendar (e.g. every 1st of the month), shared across customers.
    Calendar {
        #[map(~.into())]
        unit: CalendarUnit,
        interval: u32,
    },
    /// Fixed-length buckets anchored on the subscription activation date. Resets at every boundary.
    FixedWindow {
        #[map(~.into())]
        unit: CalendarUnit,
        interval: u32,
    },
    /// Continuous rolling window — usage older than the window edge drops out, no fixed reset.
    SlidingWindow {
        #[map(~.into())]
        unit: CalendarUnit,
        interval: u32,
    },
    /// Usage accumulates for the life of the subscription — no reset.
    Never,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::entitlements::FeatureType)]
pub enum FeatureType {
    Boolean,
    Metered {
        #[serde(with = "string_serde")]
        metric_id: BillableMetricId,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        /// Cap on usage. Null means unlimited.
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<Decimal>,
        #[serde(default = "default_reset_period")]
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        /// Percentage of the Cap at which a warning triggers (0–100).
        #[serde(skip_serializing_if = "Option::is_none")]
        warning_threshold_pct: Option<u32>,
        /// Per-entitlement kill switch. `false` means disabled.
        #[serde(default = "default_metered_enabled_rest")]
        enabled: bool,
    },
}

#[derive(Serialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EffectiveEntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        #[serde(serialize_with = "string_serde::serialize")]
        metric_id: BillableMetricId,
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<Decimal>,
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        #[serde(skip_serializing_if = "Option::is_none")]
        warning_threshold_pct: Option<u32>,
        enabled: bool,
        usage: EntitlementUsage,
    },
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
pub struct EntitlementListResponse {
    pub data: Vec<Entitlement>,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct EntitlementUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<DateTime<Utc>>,
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

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct EffectiveEntitlement {
    pub feature: FeatureRef,
    pub value: EffectiveEntitlementValue,
    /// Earliest creation timestamp across the resolved entitlements that contributed.
    pub created_at: DateTime<Utc>,
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

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct ResolvedEntitlement {
    pub feature: FeatureRef,
    pub value: ResolvedEntitlementValue,
    pub created_at: DateTime<Utc>,
    /// Highest-priority entity that contributed to the final value, with its human-readable name.
    pub origin: ResolvedOrigin,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResolvedEntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        #[serde(serialize_with = "string_serde::serialize")]
        metric_id: BillableMetricId,
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<Decimal>,
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        #[serde(skip_serializing_if = "Option::is_none")]
        warning_threshold_pct: Option<u32>,
        enabled: bool,
    },
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct ResolvedEntitlementListResponse {
    pub data: Vec<ResolvedEntitlement>,
}

/// Entitlement spec for inline attachment at feature creation time.
/// feature_id is implicit (the feature being created); the caller specifies which entity receives it.
#[derive(Serialize, Deserialize, Debug, Clone, Validate, ToSchema)]
pub struct FeatureEntitlementSpec {
    pub entity: EntitlementEntity,
    pub value: EntitlementValue,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntitlementEntity {
    Feature {
        #[serde(with = "common_domain::ids::string_serde")]
        id: FeatureId,
    },
    Plan {
        #[serde(with = "common_domain::ids::string_serde")]
        id: PlanId,
    },
    PlanVersion {
        #[serde(with = "common_domain::ids::string_serde")]
        id: PlanVersionId,
    },
    AddOn {
        #[serde(with = "common_domain::ids::string_serde")]
        id: AddOnId,
    },
    Subscription {
        #[serde(with = "common_domain::ids::string_serde")]
        id: SubscriptionId,
    },
    Quote {
        #[serde(with = "common_domain::ids::string_serde")]
        id: QuoteId,
    },
}

#[derive(Deserialize, Debug, Clone, Validate, ToSchema)]
pub struct CreateFeatureRequest {
    #[validate(length(min = 1))]
    pub name: String,
    pub description: Option<String>,
    pub feature_type: FeatureType,
    /// Product this feature belongs to. Omit for tenant-global features.
    #[serde(default, with = "common_domain::ids::string_serde_opt")]
    pub product_id: Option<ProductId>,
    /// Inline entitlement to attach when creating the feature.
    pub entitlement: Option<FeatureEntitlementSpec>,
}

#[derive(Deserialize, Debug, Clone, Validate, ToSchema)]
pub struct UpdateFeatureRequest {
    #[validate(length(min = 1))]
    pub name: Option<String>,
    /// Three-state: missing leaves unchanged, `null` clears, value sets.
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub description: Option<Option<String>>,
    /// Three-state: missing leaves unchanged, `null` clears, value sets.
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub product_id: Option<Option<ProductId>>,
}

#[derive(Deserialize, Debug, Clone, Validate, ToSchema)]
pub struct SetFeatureStatusRequest {
    pub status: FeatureStatus,
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

#[derive(Serialize, Deserialize, Debug, Clone, Validate, ToSchema)]
pub struct EntitlementSpec {
    #[serde(with = "string_serde")]
    pub feature_id: FeatureId,
    pub value: EntitlementValue,
}

#[derive(Deserialize, Debug, Clone, Validate, ToSchema)]
pub struct UpdateEntitlementRequest {
    /// Replace the entitlement value entirely.
    pub value: Option<EntitlementValue>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_feature_description_three_states() {
        let missing: UpdateFeatureRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.description, None);

        let null: UpdateFeatureRequest = serde_json::from_str(r#"{"description": null}"#).unwrap();
        assert_eq!(null.description, Some(None));

        let value: UpdateFeatureRequest =
            serde_json::from_str(r#"{"description": "hello"}"#).unwrap();
        assert_eq!(value.description, Some(Some("hello".to_string())));
    }
}
