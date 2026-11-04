use common_domain::ids::{
    BillableMetricId, EntitlementId, FeatureId, ProductId, string_serde, string_serde_opt,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};
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
    /// Hidden with its entitlements until unarchived. Its code is free for a new feature.
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
/// A sliding window has no reset moment (`reset_at` is absent) and sends no threshold or
/// limit-reached webhooks; it is enforced by the entitlement check.
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

pub use meteroid_rest_common::entitlements::{
    BooleanConfigValue, ConfigValue, JsonConfigValue, NumberConfigValue, TextConfigValue,
};

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct BooleanFeatureType {}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct MeteredFeatureType {
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
}

/// Authoritative value type of a Config feature. `MAP`/`JSON` both carry a JSON value.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConfigValueType {
    Number,
    Boolean,
    Text,
    Map,
    Json,
    Select,
}

/// A static, typed configuration value. No metric — resolved synchronously.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ConfigFeatureType {
    /// The feature's value type, fixed at creation.
    pub value_type: ConfigValueType,
    /// Allowed values when `value_type = SELECT`. Empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FeatureType {
    Boolean(BooleanFeatureType),
    Metered(MeteredFeatureType),
    Config(ConfigFeatureType),
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
            meteroid_store::domain::entitlements::FeatureType::Config { value_type } => {
                use meteroid_store::domain::entitlements::ConfigValueType as D;
                let (value_type, options) = match value_type {
                    D::Number => (ConfigValueType::Number, vec![]),
                    D::Boolean => (ConfigValueType::Boolean, vec![]),
                    D::Text => (ConfigValueType::Text, vec![]),
                    D::Map => (ConfigValueType::Map, vec![]),
                    D::Json => (ConfigValueType::Json, vec![]),
                    D::Select { options } => (ConfigValueType::Select, options),
                };
                FeatureType::Config(ConfigFeatureType {
                    value_type,
                    options,
                })
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct BooleanEntitlementValue {
    pub enabled: bool,
}

/// What happens once usage reaches the limit.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(as = EntitlementOverageBehavior)]
pub enum OverageBehavior {
    /// Usage is refused once it reaches the limit, plus `grace_pct` if set.
    Block,
    /// Usage continues past the limit; the overage is billed by a price on the same metric.
    Allow,
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
    /// What happens at the limit; requires a limit.
    ///
    /// The limit terms (`overage_behavior`, `grace_pct`, `thresholds`, `group_by`) follow one
    /// rule: omitted, a term keeps its current value on update (its default on create); `null`
    /// sets it to its default (`BLOCK`, no grace, no thresholds, no grouping). An omitted term
    /// that no longer applies (no limit, or a sliding window for the thresholds) falls back to its
    /// default; one sent explicitly is refused with 400.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<OverageBehavior>)]
    pub overage_behavior: Option<Option<OverageBehavior>>,
    /// `BLOCK` only: usage allowed beyond the limit before it is refused, in percent of the
    /// limit (1 to 1000).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<u32>)]
    pub grace_pct: Option<Option<u32>>,
    /// Deprecated: use `thresholds`. A single alert threshold, read only when `thresholds` is
    /// omitted; returned as the lowest of `thresholds`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<u32>)]
    pub warning_threshold_pct: Option<Option<u32>>,
    /// Percents of the limit (1 to 100, at most 10) at which an alert is sent, once per period:
    /// `entitlement.usage_threshold_reached`, or `entitlement.limit_reached` at 100. Requires a
    /// limit and a window that is not sliding.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<Vec<u32>>)]
    pub thresholds: Option<Option<Vec<u32>>>,
    /// Event property the limit applies to separately, one limit per value (e.g. `user_id`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<String>)]
    pub group_by: Option<Option<String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ConfigEntitlementValue {
    pub value: ConfigValue,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntitlementValue {
    Boolean(BooleanEntitlementValue),
    Metered(MeteredEntitlementValue),
    Config(ConfigEntitlementValue),
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct MeteredEntitlementSpec {
    #[serde(serialize_with = "string_serde::serialize")]
    pub metric_id: BillableMetricId,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    pub reset_period: ResetPeriod,
    /// False when switched off, or once a `BLOCK` limit (plus its grace) is reached. An `ALLOW`
    /// limit never turns it off.
    pub enabled: bool,
    /// What happens at the limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overage_behavior: Option<OverageBehavior>,
    /// `BLOCK` only: usage allowed beyond the limit before it is refused, in percent of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_pct: Option<u32>,
    /// Deprecated: the lowest of `thresholds`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_threshold_pct: Option<u32>,
    /// Percents of the limit at which an alert is sent, ascending.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<Vec<u32>>,
    /// Event property the limit applies to separately; usage is then the largest value's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
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
    pub reset_at: Option<ApiDateTime>,
    /// Switched on, but refused: the `BLOCK` limit (plus its grace) is reached, so `enabled` is
    /// false until the window resets. Never set for a limit grouped by a property.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_reached: Option<bool>,
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
pub struct ConfigEffectiveEntitlementValue {
    pub value: ConfigValue,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EffectiveEntitlementValue {
    Boolean(BooleanEffectiveEntitlementValue),
    Metered(MeteredEffectiveEntitlementValue),
    Config(ConfigEffectiveEntitlementValue),
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
    /// What happens at the limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overage_behavior: Option<OverageBehavior>,
    /// `BLOCK` only: usage allowed beyond the limit before it is refused, in percent of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_pct: Option<u32>,
    /// Deprecated: the lowest of `thresholds`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_threshold_pct: Option<u32>,
    /// Percents of the limit at which an alert is sent, ascending.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<Vec<u32>>,
    /// Event property the limit applies to separately; usage is then the largest value's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct ConfigResolvedEntitlementValue {
    pub value: ConfigValue,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResolvedEntitlementValue {
    Boolean(BooleanResolvedEntitlementValue),
    Metered(MeteredResolvedEntitlementValue),
    Config(ConfigResolvedEntitlementValue),
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct Feature {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: FeatureId,
    pub name: String,
    /// Unique key used to reference this feature in your code. Cannot be changed after creation.
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub feature_type: FeatureType,
    pub status: FeatureStatus,
    /// Product this feature belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    pub created_at: ApiDateTime,
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
    pub created_at: ApiDateTime,
    pub updated_at: ApiDateTime,
}

/// Minimal reference to the product a feature belongs to.
#[derive(Serialize, Debug, Clone, ToSchema)]
#[schema(as = EntitlementProductRef)]
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
    /// Unique key used to reference this feature in your code. Cannot be changed after creation.
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
}

/// Merged entitlement value for a feature for a specific customer, enriched with live usage data.
#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct EffectiveEntitlement {
    pub feature: FeatureRef,
    pub value: EffectiveEntitlementValue,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct EffectiveEntitlementListResponse {
    pub data: Vec<EffectiveEntitlement>,
}

/// Merged entitlement value for a feature across the priority hierarchy, without usage data.
#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct ResolvedEntitlement {
    pub feature: FeatureRef,
    pub value: ResolvedEntitlementValue,
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
    #[serde(default, with = "string_serde_opt")]
    pub product_id: Option<ProductId>,
    /// Search by feature name.
    pub search: Option<String>,
}

/// Mirrors the gRPC feature-code rule so both APIs accept the same slugs.
fn validate_feature_code(code: &str) -> Result<(), validator::ValidationError> {
    if lazy_regex::regex_is_match!(r"^[a-z0-9][a-z0-9_-]*$", code) {
        return Ok(());
    }
    let mut err = validator::ValidationError::new("invalid_feature_code");
    err.message = Some(
        "must start with a lowercase letter or digit, then only lowercase letters, digits, '-', '_'"
            .into(),
    );
    Err(err)
}

/// A SELECT feature with no options can never be satisfied, and options are fixed at creation.
fn validate_feature_type(feature_type: &FeatureType) -> Result<(), validator::ValidationError> {
    let FeatureType::Config(config) = feature_type else {
        return Ok(());
    };
    if !matches!(config.value_type, ConfigValueType::Select) || !config.options.is_empty() {
        return Ok(());
    }
    let mut err = validator::ValidationError::new("empty_select_options");
    err.message = Some("a SELECT config feature requires at least one option".into());
    Err(err)
}

/// Mirrors the store's metered-value rule so the request fails before any read or write.
fn validate_entitlement_value(value: &EntitlementValue) -> Result<(), validator::ValidationError> {
    let EntitlementValue::Metered(metered) = value else {
        return Ok(());
    };
    if metered.limit.is_some_and(|l| l.is_sign_negative()) {
        let mut err = validator::ValidationError::new("negative_limit");
        err.message = Some("limit must not be negative".into());
        return Err(err);
    }
    if metered.overage_behavior == Some(Some(OverageBehavior::Allow))
        && matches!(metered.grace_pct, Some(Some(_)))
    {
        let mut err = validator::ValidationError::new("grace_requires_block");
        err.message = Some("grace_pct applies to the BLOCK overage behavior only".into());
        return Err(err);
    }
    let interval = match &metered.reset_period {
        ResetPeriod::Calendar(p) => p.interval,
        ResetPeriod::FixedWindow(p) => p.interval,
        ResetPeriod::SlidingWindow(p) => p.interval,
        ResetPeriod::BillingCycle(_) | ResetPeriod::Never(_) => return Ok(()),
    };
    if interval == 0 {
        let mut err = validator::ValidationError::new("zero_interval");
        err.message = Some("reset_period.interval must be at least 1".into());
        return Err(err);
    }
    Ok(())
}

/// Two specs for one feature would race on the entity's unique key and surface as a 409.
pub fn validate_unique_feature_ids(
    specs: &[EntitlementSpecRequest],
) -> Result<(), validator::ValidationError> {
    let mut seen = std::collections::HashSet::with_capacity(specs.len());
    for spec in specs {
        if !seen.insert(spec.feature_id) {
            let mut err = validator::ValidationError::new("duplicate_feature_id");
            err.message =
                Some(format!("feature {} appears more than once", spec.feature_id).into());
            return Err(err);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct CreateFeatureRequest {
    #[validate(length(min = 1))]
    pub name: String,
    /// Unique key used to reference this feature in your code. Cannot be changed after creation.
    #[validate(length(min = 1, max = 128), custom(function = "validate_feature_code"))]
    pub code: String,
    pub description: Option<String>,
    /// Fixed at creation — a feature never changes type.
    #[validate(custom(function = "validate_feature_type"))]
    pub feature_type: FeatureType,
    /// Product this feature belongs to. Omit for a tenant-global feature.
    #[serde(default, with = "string_serde_opt")]
    pub product_id: Option<ProductId>,
    /// Feature-level (default) entitlement, applied to all subscriptions as the lowest-priority.
    #[validate(custom(function = "validate_entitlement_value"))]
    pub entitlement: Option<EntitlementValue>,
}

/// Partial update. Code, feature type and product are immutable.
#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateFeatureRequest {
    #[validate(length(min = 1))]
    pub name: Option<String>,
    /// Omit to leave unchanged; send `null` to clear.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateEntitlementRequest {
    /// Omit to leave the entitlement unchanged. Must match the feature's type.
    #[validate(custom(function = "validate_entitlement_value"))]
    pub value: Option<EntitlementValue>,
}

/// One entitlement to create: which feature, and the value granted by the entity.
// `Serialize` is only here because `validator` echoes a rejected field's value back into the
// validation error params.
#[derive(Clone, Debug, Serialize, Deserialize, Validate, ToSchema)]
pub struct EntitlementSpecRequest {
    #[serde(with = "string_serde")]
    pub feature_id: FeatureId,
    #[validate(custom(function = "validate_entitlement_value"))]
    pub value: EntitlementValue,
}

/// Entitlements already present on the entity are skipped, so the call can be replayed.
#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct CreateEntitlementsRequest {
    #[validate(
        length(min = 1),
        nested,
        custom(function = "validate_unique_feature_ids")
    )]
    pub entitlements: Vec<EntitlementSpecRequest>,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct EntitlementListResponse {
    pub data: Vec<Entitlement>,
}
