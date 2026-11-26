use common_domain::ids::{
    AliasOr, BillableMetricId, EntitlementId, FeatureId, ProductId, ResourceId, ResourceTypeId,
    SubscriptionId, UsageLimitId, string_serde, string_serde_opt,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

fn default_true() -> bool {
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

/// A limit: no `limit` is unlimited, `enabled: false` is off. Resets, behavior at the limit and
/// thresholds are the feature's `policy` or a Set to row's `policy_override`. The deprecated terms
/// are read on writes: a Set to row takes them as its override, others refuse them unless equal.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct MeteredEntitlementValue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Deprecated: the policy's `resets` and `period`. Write-only, like the terms below: one
    /// omitted keeps the policy in effect; `null` sets the others to their default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(deprecated)]
    pub reset_period: Option<ResetPeriod>,
    /// Deprecated: the policy's `at_limit` (`ALLOW` is `ALLOW_AND_BILL`, `BLOCK` is `DENY`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<OverageBehavior>, deprecated)]
    pub overage_behavior: Option<Option<OverageBehavior>>,
    /// Deprecated: the policy's `grace_pct`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<u32>, deprecated)]
    pub grace_pct: Option<Option<u32>>,
    /// Deprecated: a single threshold, read only when `thresholds` is omitted or empty.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<u32>, deprecated)]
    pub warning_threshold_pct: Option<Option<u32>>,
    /// Deprecated: the policy's `thresholds`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<Vec<u32>>, deprecated)]
    pub thresholds: Option<Option<Vec<u32>>>,
    /// Deprecated: no longer supported, a limit applies per resource instead (`level`). Only
    /// `null` is accepted.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[schema(value_type = Option<String>, deprecated)]
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

/// A limit counted by your app: its value and the count reported.
#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct AppCountedEffectiveEntitlementValue {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub used: Option<Decimal>,
}

/// A value feature the customer isn't entitled to, or one read without the resource it needs.
#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct NotEntitledEffectiveEntitlementValue {}

/// SDKs that predate a variant read it as an unknown value rather than failing the response.
#[derive(Serialize, Debug, Clone, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EffectiveEntitlementValue {
    Boolean(BooleanEffectiveEntitlementValue),
    Metered(MeteredEffectiveEntitlementValue),
    Config(ConfigEffectiveEntitlementValue),
    AppCounted(AppCountedEffectiveEntitlementValue),
    NotEntitled(NotEntitledEffectiveEntitlementValue),
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FeatureKind {
    /// On or off.
    Boolean,
    /// A number, text, select or json value.
    Value,
    /// An amount, unlimited, or off.
    Limit,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FeatureValueType {
    Number,
    Text,
    Select,
    Json,
}

/// What a `VALUE` feature holds.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct FeatureValueSpec {
    #[serde(rename = "type")]
    pub value_type: FeatureValueType,
    /// Numbers: which way is better, for Raise to and Cap at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub higher_is_better: Option<bool>,
    /// Select: the allowed values, worst first when `ordered`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default)]
    pub ordered: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LimitCountedBy {
    /// Meteroid counts the events of `metric_id`.
    Events,
    /// Your app reports the count, with `POST /customers/{id}/entitlements/{feature}/usage`.
    App,
}

/// How a `LIMIT` feature is counted.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct FeatureLimitSpec {
    pub counted_by: LimitCountedBy,
    #[serde(
        default,
        with = "string_serde_opt",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub metric_id: Option<BillableMetricId>,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::entitlement_policy::Resets)]
pub enum PolicyResets {
    BillingCycle,
    /// On calendar boundaries of `period` (each day at midnight UTC, each month on the 1st, …).
    Calendar,
    /// Every `period`, from the activation.
    Fixed,
    /// Over the last `period`.
    Sliding,
    Never,
}

/// The length of a calendar, fixed or sliding window.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[map_owned(meteroid_store::domain::entitlement_policy::Period)]
pub struct PolicyPeriod {
    #[map(~.into())]
    pub unit: CalendarUnit,
    pub interval: u32,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::entitlement_policy::AtLimit)]
pub enum PolicyAtLimit {
    /// Usage beyond the limit plus `grace_pct` is refused.
    Deny,
    /// Usage continues; the overage is billed by a price on the same metric.
    AllowAndBill,
    /// Limits counted by your app: what exists stays, nothing new is allowed.
    DenyNew,
}

/// A limit's policy. A limit counted by your app never resets and is `DENY_NEW`.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[map_owned(meteroid_store::domain::entitlement_policy::FeaturePolicy)]
pub struct FeaturePolicy {
    #[map(~.into())]
    pub resets: PolicyResets,
    /// `CALENDAR`, `FIXED` and `SLIDING` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[map(~.map(Into::into))]
    pub period: Option<PolicyPeriod>,
    #[map(~.into())]
    pub at_limit: PolicyAtLimit,
    #[serde(default)]
    pub grace_pct: u32,
    /// Percents of the limit at which an alert is sent, ascending.
    #[serde(default)]
    pub thresholds: Vec<u32>,
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
    pub kind: FeatureKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<FeatureValueSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<FeatureLimitSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The resource type the feature applies per; absent: the customer.
    #[serde(
        serialize_with = "string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub level: Option<ResourceTypeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<FeaturePolicy>,
    /// Deprecated: read `kind`; absent for a limit counted by your app. Removed once every client
    /// reads `kind`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_type: Option<FeatureType>,
    pub status: FeatureStatus,
    /// Product this feature belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    pub created_at: ApiDateTime,
    /// Deprecated: the first of `defaults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entitlement: Option<Entitlement>,
    /// The feature's default rows, the lowest tier.
    pub defaults: Vec<Entitlement>,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct FeatureListResponse {
    pub data: Vec<Feature>,
    pub pagination_meta: PaginationResponse,
}

/// How a row combines with the value below it.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::entitlement_fold::Effect)]
pub enum EntitlementEffect {
    Set,
    /// Add-ons: adds the value times the add-on's quantity.
    Add,
    RaiseTo,
    CapAt,
}

/// `FEATURE` follows the feature's level, `CUSTOMER` is across all the customer's resources,
/// `TYPE` is per resource of `level_type_id`.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Default, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntitlementLevel {
    #[default]
    Feature,
    Customer,
    Type,
}

/// `{"<type code>.<attribute>": ["value", …]}`: the row applies when every attribute holds one of
/// its values.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, ToSchema)]
#[serde(transparent)]
pub struct EntitlementCondition(pub BTreeMap<String, Vec<String>>);

pub use crate::webhooks::out_model::WebhookOutActor as EntitlementActor;

/// One row. `id` is its lineage: an edit revokes the row and writes a successor under the same id.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct Entitlement {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EntitlementId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub feature_id: FeatureId,
    pub value: EntitlementValue,
    pub effect: EntitlementEffect,
    pub level: EntitlementLevel,
    #[serde(
        serialize_with = "string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub level_type_id: Option<ResourceTypeId>,
    /// A customer override on one stored resource.
    #[serde(
        serialize_with = "string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub resource_id: Option<ResourceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<EntitlementCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_override: Option<FeaturePolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<EntitlementActor>,
    /// When the lineage was created.
    pub created_at: ApiDateTime,
    /// When this row was written.
    pub updated_at: ApiDateTime,
    /// History rows: when an edit replaced the row, or it was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<String>,
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
    /// On a subscription's entitlements, when another subscription decides the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_grant: Option<SubscriptionGrant>,
}

/// What a subscription's own rows grant on a level another subscription decides: "grants 1k;
/// Pro on the customer decides 10k".
#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct SubscriptionGrant {
    /// Absent when the subscription includes the feature through its product alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
    #[serde(serialize_with = "string_serde::serialize")]
    #[schema(value_type = String)]
    pub deciding_subscription_id: SubscriptionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deciding_plan_name: Option<String>,
    /// The deciding subscription's resource (`site=s1`); absent for the customer's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deciding_resource_path: Option<String>,
}

#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct EffectiveEntitlementListResponse {
    pub data: Vec<EffectiveEntitlement>,
    /// New: `usage_unavailable` when metering could not answer for an entitlement's usage, whose
    /// `consumed` is then absent.
    pub warnings: Vec<String>,
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

fn validation_error(code: &'static str, message: &'static str) -> validator::ValidationError {
    let mut err = validator::ValidationError::new(code);
    err.message = Some(message.into());
    err
}

/// The kind takes its own spec only, and a select needs options.
fn validate_kind(request: &CreateFeatureRequest) -> Result<(), validator::ValidationError> {
    let Some(kind) = request.kind else {
        return match &request.feature_type {
            Some(FeatureType::Config(c))
                if matches!(c.value_type, ConfigValueType::Select) && c.options.is_empty() =>
            {
                Err(validation_error(
                    "empty_select_options",
                    "a SELECT config feature requires at least one option",
                ))
            }
            Some(_) => Ok(()),
            None => Err(validation_error("kind_required", "`kind` is required")),
        };
    };
    match (kind, &request.value, &request.limit) {
        (FeatureKind::Boolean, None, None) | (FeatureKind::Limit, None, Some(_)) => Ok(()),
        (FeatureKind::Value, Some(v), None)
            if v.value_type != FeatureValueType::Select || !v.options.is_empty() =>
        {
            Ok(())
        }
        (FeatureKind::Value, Some(_), None) => Err(validation_error(
            "empty_select_options",
            "a SELECT value requires at least one option",
        )),
        _ => Err(validation_error(
            "kind_spec_mismatch",
            "VALUE takes `value`, LIMIT takes `limit`, BOOLEAN neither",
        )),
    }
}

fn validate_entitlement_value(value: &EntitlementValue) -> Result<(), validator::ValidationError> {
    match value {
        EntitlementValue::Metered(m) if m.limit.is_some_and(|l| l.is_sign_negative()) => Err(
            validation_error("negative_limit", "limit must not be negative"),
        ),
        _ => Ok(()),
    }
}

/// Two specs for one (feature, level, condition) would collide on the owner's open rows.
pub fn validate_unique_feature_ids(
    specs: &[EntitlementSpecRequest],
) -> Result<(), validator::ValidationError> {
    let mut seen = std::collections::HashSet::with_capacity(specs.len());
    for spec in specs {
        let key = (
            spec.feature_id,
            spec.level,
            spec.level_type_id,
            spec.condition.clone(),
        );
        if !seen.insert(key) {
            let mut err = validation_error("duplicate_entitlement", "");
            err.message = Some(
                format!(
                    "feature {} appears twice with the same level and condition",
                    spec.feature_id
                )
                .into(),
            );
            return Err(err);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
#[validate(schema(function = "validate_kind"))]
pub struct CreateFeatureRequest {
    #[validate(length(min = 1))]
    pub name: String,
    /// Unique key used to reference this feature in your code. Cannot be changed after creation.
    #[validate(length(min = 1, max = 128), custom(function = "validate_feature_code"))]
    pub code: String,
    pub description: Option<String>,
    /// Fixed at creation. Required, unless the deprecated `feature_type` is sent.
    pub kind: Option<FeatureKind>,
    /// `VALUE` only.
    pub value: Option<FeatureValueSpec>,
    /// `LIMIT` only.
    pub limit: Option<FeatureLimitSpec>,
    /// Deprecated: send `kind`. `METERED` is a `LIMIT` counted from events, a `CONFIG` boolean is
    /// `BOOLEAN`, any other `CONFIG` a `VALUE` (`MAP` is `JSON`). With `kind`, both must agree.
    #[schema(deprecated)]
    pub feature_type: Option<FeatureType>,
    pub unit: Option<String>,
    /// The resource type the feature applies per (`rty_…`). Omit for the customer.
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub level: Option<ResourceTypeId>,
    /// Limits only.
    pub policy: Option<FeaturePolicy>,
    /// Product this feature belongs to. Omit for a tenant-global feature.
    #[serde(default, with = "string_serde_opt")]
    pub product_id: Option<ProductId>,
    /// Feature-level (default) entitlement, applied to all subscriptions as the lowest-priority.
    #[validate(custom(function = "validate_entitlement_value"))]
    pub entitlement: Option<EntitlementValue>,
}

/// Partial update. Code, kind and product are immutable.
#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateFeatureRequest {
    #[validate(length(min = 1))]
    pub name: Option<String>,
    /// Omit to leave unchanged; send `null` to clear.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub unit: Option<Option<String>>,
    /// Moves the rows on the feature's level with it. `null`: the customer.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub level: Option<Option<ResourceTypeId>>,
    pub policy: Option<FeaturePolicy>,
    /// Numbers only.
    pub higher_is_better: Option<bool>,
    /// Selects only. An option a row still uses can't be removed.
    pub select_options: Option<Vec<String>>,
    pub select_ordered: Option<bool>,
}

/// An omitted field keeps its stored value; `null` clears it. Owner, feature, level and resource
/// stay: delete the row and create another to change them.
#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateEntitlementRequest {
    /// Must match the feature's kind.
    #[validate(custom(function = "validate_entitlement_value"))]
    pub value: Option<EntitlementValue>,
    pub effect: Option<EntitlementEffect>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<EntitlementCondition>)]
    pub condition: Option<Option<EntitlementCondition>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<ApiDateTime>)]
    pub valid_from: Option<Option<ApiDateTime>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<ApiDateTime>)]
    pub valid_until: Option<Option<ApiDateTime>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub reason: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<FeaturePolicy>)]
    pub policy_override: Option<Option<FeaturePolicy>>,
}

/// One row to create. `effect` defaults to the source's: Set to for defaults, plans and
/// subscriptions, Add (several instances) or Raise to for add-ons, Raise to for overrides.
// `Serialize` is only here because `validator` echoes a rejected field's value back into the
// validation error params.
#[derive(Clone, Debug, Serialize, Deserialize, Validate, ToSchema)]
pub struct EntitlementSpecRequest {
    #[serde(with = "string_serde")]
    pub feature_id: FeatureId,
    #[validate(custom(function = "validate_entitlement_value"))]
    pub value: EntitlementValue,
    pub effect: Option<EntitlementEffect>,
    #[serde(default)]
    pub level: EntitlementLevel,
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub level_type_id: Option<ResourceTypeId>,
    pub condition: Option<EntitlementCondition>,
    pub valid_from: Option<ApiDateTime>,
    pub valid_until: Option<ApiDateTime>,
    pub reason: Option<String>,
    /// Set to rows only.
    pub policy_override: Option<FeaturePolicy>,
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

/// Customer overrides. With `resource_id`, every row overrides that stored resource of the
/// customer. Unlike the other owners, a row clashing with an open one is a 409.
#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct CreateCustomerEntitlementsRequest {
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub resource_id: Option<ResourceId>,
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

/// Every row of the entitlement, oldest first.
#[derive(Serialize, Debug, Clone, ToSchema)]
pub struct EntitlementHistoryResponse {
    pub data: Vec<Entitlement>,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct CustomerFeaturePath {
    #[validate(nested)]
    pub id_or_alias: AliasOr<common_domain::ids::CustomerId>,
    #[validate(nested)]
    pub feature_id_or_code: AliasOr<FeatureId>,
}

/// The query of the check's `GET` alias. `resources` and `attributes` are passed as
/// `resources[<key>]=<value>` and `attributes[<type>.<attribute>]=<value>`.
#[derive(Debug, Clone, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EntitlementCheckQuery {
    /// Units about to be consumed. Defaults to 1.
    #[param(value_type = Option<String>, format = "decimal")]
    pub requested: Option<Decimal>,
    /// Limits counted by your app: replaces the stored count for this request. Server key only.
    #[param(value_type = Option<String>, format = "decimal")]
    pub current_count: Option<Decimal>,
    /// Adds how each level's value was built. Server key only.
    #[serde(default)]
    pub explain: bool,
    /// Deprecated, ignored: the subscription deciding the feature answers.
    #[serde(default, with = "string_serde_opt")]
    #[param(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
}

#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
pub struct EntitlementCheckRequest {
    /// Feature id or code.
    #[validate(nested)]
    #[schema(value_type = String)]
    pub feature: AliasOr<FeatureId>,
    /// At most one resource, by any of its type's keys: `{"site_id": "s1"}`.
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
    /// Asserted attributes, `{"site.environment": "staging"}`. A stored resource's own win.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// Units about to be consumed. Defaults to 1.
    #[schema(value_type = Option<String>, format = "decimal")]
    pub requested: Option<Decimal>,
    /// Limits counted by your app: replaces the stored count for this request. Server key only.
    #[schema(value_type = Option<String>, format = "decimal")]
    pub current_count: Option<Decimal>,
    /// Adds how each level's value was built. Server key only.
    #[serde(default)]
    pub explain: bool,
    /// Deprecated, ignored: the subscription deciding the feature answers.
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
    /// Any other field; the dropped ones (`as_of`, `group`, …) are refused.
    #[serde(flatten)]
    #[schema(ignore)]
    pub other: BTreeMap<String, serde_json::Value>,
}

/// Inputs no check takes any more (ADR 0011 §3.1, §3.12): a 400 rather than an answer that
/// silently ignores them.
pub const DROPPED_CHECK_INPUTS: &[&str] = &["as_of", "group", "group_by"];

impl EntitlementCheckRequest {
    /// The first dropped input among `keys`, as `group[region]` or `as_of`.
    pub fn dropped_input<'a>(keys: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
        keys.into_iter().find(|key| {
            let name = key.split('[').next().unwrap_or(key);
            DROPPED_CHECK_INPUTS.contains(&name)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntitlementCheckReason {
    /// Every level allows the request.
    WithinLimit,
    /// A level's limit would be exceeded by the request.
    LimitReached,
    /// A level grants nothing.
    NoEntitlement,
    /// A level's entitlement is switched off: a boolean off, or a limit set to off.
    Disabled,
    /// A hard budget covering the feature's usage is spent.
    BudgetExceeded,
    /// The customer pays the feature's usage from prepaid credit, and none is left.
    InsufficientBalance,
    /// Never sent: limits are no longer grouped.
    TooManyGroups,
    /// The feature applies per resource and the check names none of its type.
    ResourceRequired,
    /// The check's resource has no subscription including the feature.
    NotSubscribed,
    /// The metric a level's limit counts cannot be loaded, so it is not allowed until the
    /// feature is fixed; `reason_type` is `misconfigured`.
    MetricNotFound,
    /// Metering could not answer for the usage a level's limit is measured against, so it is not
    /// allowed; `reason_type` is `unavailable` and `retry_after_ms` a few seconds.
    UsageUnavailable,
}

// Kept for the published schema of the deprecated, always empty `EntitlementCheck::limits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntitlementCheckLimitBehavior {
    /// Usage is recorded against the limit only, silently: no webhook or email is sent.
    Track,
    /// An alert is sent at each threshold; usage is never refused.
    Soft,
    /// Usage beyond the limit plus its grace is refused.
    Hard,
}

/// Deprecated: customer usage limits were replaced by entitlement overrides; never returned.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntitlementCheckLimit {
    #[serde(serialize_with = "string_serde::serialize")]
    pub usage_limit_id: UsageLimitId,
    pub behavior: EntitlementCheckLimitBehavior,
    #[schema(value_type = String, format = "decimal")]
    pub used: Decimal,
    #[schema(value_type = String, format = "decimal")]
    pub limit: Decimal,
    #[schema(value_type = String, format = "decimal")]
    pub remaining: Decimal,
    /// The group the limit was evaluated for, when the limit is grouped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<BTreeMap<String, String>>,
    pub has_access: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntitlementCheckPeriod {
    pub start: ApiDateTime,
    /// Absent when the period never ends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<ApiDateTime>,
}

#[derive(o2o, Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[from_owned(meteroid_store::domain::entitlement_levels::ReasonType)]
pub enum EntitlementCheckReasonType {
    Denied,
    /// An integration error, such as a missing resource or a metric not found: fix the call or the
    /// feature, not the plan; retrying does not help.
    Misconfigured,
    /// New: usage could not be read in time, no decision about the customer; retry after
    /// `retry_after_ms`.
    Unavailable,
}

/// Where the usage came from: `read` when checking, else `live`, kept up to date by the live
/// engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Read,
    Live,
}

impl From<meteroid_store::domain::entitlement_checks::UsageSource> for UsageSource {
    fn from(source: meteroid_store::domain::entitlement_checks::UsageSource) -> Self {
        use meteroid_store::domain::entitlement_checks::UsageSource as Domain;
        match source {
            Domain::Read => UsageSource::Read,
            Domain::Monitor => UsageSource::Live,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntitlementCheckFeature {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: FeatureId,
    pub code: String,
}

#[derive(o2o, Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[from_owned(meteroid_store::domain::entitlement_fold::Tier)]
pub enum EntitlementTier {
    Default,
    Plan,
    PlanVersion,
    Subscription,
    AddOn,
    Customer,
    ResourceOverride,
}

#[derive(o2o, Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[from_owned(meteroid_store::domain::entitlement_fold::Discarded)]
pub enum DiscardReason {
    ReplacedByHigherTier,
    LostWithinTier,
    NoEffect,
    /// An Add or Raise to on a level with nothing to add to or raise.
    NoBaseOnLevel,
    EffectNotAllowed,
    ConditionNotMet,
    NotInEffect,
}

/// One row's part in a level's value.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntitlementContribution {
    #[serde(serialize_with = "string_serde::serialize")]
    pub entitlement_id: EntitlementId,
    /// The entity holding the row, as `<type>:<id>`.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    pub tier: EntitlementTier,
    pub effect: EntitlementEffect,
    pub value: EntitlementValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<EntitlementCondition>,
    pub quantity: u32,
    /// A limit counted per unit of a managed quantity: the units held, which multiply `value`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<u32>,
    pub policy_override: bool,
    /// The value after this row; absent when it was discarded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<EntitlementValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discarded: Option<DiscardReason>,
}

/// A limit counted by your app, per path: the exact count, the last set, and the adds since.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppCountExplain {
    pub path: String,
    #[schema(value_type = String, format = "decimal")]
    pub count: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub last_set_value: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_set_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_set_key: Option<String>,
    pub adds_since: i64,
    #[schema(value_type = String, format = "decimal")]
    pub adds_sum: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_add_at: Option<ApiDateTime>,
}

/// A subscription including the feature at the level, with the value its rows give there.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DecidingCandidate {
    #[serde(serialize_with = "string_serde::serialize")]
    #[schema(value_type = String)]
    pub subscription_id: SubscriptionId,
    /// Absent for the whole customer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LevelExplain {
    pub contributions: Vec<EntitlementContribution>,
    pub candidates: Vec<DecidingCandidate>,
    pub app_counts: Vec<AppCountExplain>,
}

/// A feature at one level of the path: the customer, or the resource the request names.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntitlementLevelState {
    /// The level's resource type code, e.g. `site`; absent for the customer level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// `""` for the customer, else `<type code>=<key value>`.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub used: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    pub near_limit: bool,
    /// Past the limit of a level that allows and bills.
    pub in_overage: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<ApiDateTime>,
    #[serde(
        serialize_with = "string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub deciding_subscription_id: Option<SubscriptionId>,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<EntitlementCheckReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<LevelExplain>,
}

/// Whether a customer may consume more of a feature now. Every level on the path must allow.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntitlementCheck {
    pub feature: EntitlementCheckFeature,
    pub allowed: bool,
    /// Same as `allowed`.
    pub has_access: bool,
    pub reason: EntitlementCheckReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_type: Option<EntitlementCheckReasonType>,
    pub levels: Vec<EntitlementLevelState>,
    /// `resource_omitted`: customer subscriptions answered while resource ones include the feature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub attributes_ignored: Vec<String>,
    pub warnings: Vec<String>,
    pub usage_source: UsageSource,
    /// `used`, `limit`, `remaining` and `period` report the level that denies, else the feature's
    /// own. Absent without a limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub used: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    /// Units left before the limit, grace excluded.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub remaining: Option<Decimal>,
    /// The current period of the limit's usage; absent for a limit counted by your app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<EntitlementCheckPeriod>,
    /// When the answer was given; while usage cannot be read, when the live engine last counted the
    /// usage that answered.
    pub as_of: ApiDateTime,
    /// Deprecated: always 0.
    pub lag_ms: i64,
    /// True when the usage was read when checking.
    pub exact: bool,
    /// True when a level is within 5% of its hard limit or past it (used against the limit). While
    /// true, check before every request and do not cache an answer that grants access.
    pub near_limit: bool,
    /// When to check again, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<i64>,
    /// Deprecated: always empty. Customer-specific limits are entitlement overrides, reported in
    /// `used`, `limit` and `remaining`.
    pub limits: Vec<EntitlementCheckLimit>,
    /// A limit that allows overage is exceeded by the request: access is granted, and billed.
    pub in_overage: bool,
    /// The value of a `VALUE` feature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ConfigValue>,
}

#[derive(o2o, Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[from_owned(meteroid_store::domain::entitlement_checks::FeatureScope)]
pub enum FeatureScope {
    Customer,
    /// Only resource subscriptions include the feature: it is answered per resource.
    Resource,
}

/// A feature at one level, as the snapshot reads it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SnapshotLevel {
    /// The level's resource type code, e.g. `site`; absent for the customer level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// `""` for the customer, else `<type code>=<key value>`.
    pub path: String,
    /// Absent without a value: see `reason`; else nothing is imposed, or on the feature's own
    /// level, it is not entitled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
    /// `RESOURCE_REQUIRED` or `NOT_SUBSCRIBED` when the level needs another resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<EntitlementCheckReason>,
    pub policy: FeaturePolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub used: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<ApiDateTime>,
    #[serde(
        serialize_with = "string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub deciding_subscription_id: Option<SubscriptionId>,
    /// The row that set the value, with its source's name; absent without a value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<EntitlementContribution>,
}

/// `feature` and `value` keep the shape of the earlier list; `scope` and `levels` add each level.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SnapshotEntitlement {
    pub feature: FeatureRef,
    /// The feature's own level's value capped by the others, with its usage: always present.
    pub value: EffectiveEntitlementValue,
    pub scope: FeatureScope,
    pub levels: Vec<SnapshotLevel>,
}

/// Every feature with its levels. `etag` is also the `ETag` header: send it back as
/// `If-None-Match` to get a 304 while nothing changed.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntitlementSnapshot {
    pub etag: String,
    pub data: Vec<SnapshotEntitlement>,
    /// Asserted attributes the stored resource's own overrode.
    pub attributes_ignored: Vec<String>,
    /// `usage_unavailable`: a level's usage could not be read, its `used` is absent.
    pub warnings: Vec<String>,
}

/// Reports a limit counted by your app: `set` the count, or `add` to it (negative to remove).
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
pub struct ReportUsageRequest {
    #[schema(value_type = Option<String>, format = "decimal")]
    pub set: Option<Decimal>,
    #[schema(value_type = Option<String>, format = "decimal")]
    pub add: Option<Decimal>,
    /// The resource counted, by any of its type's keys. Omit for the customer.
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
    /// When the change happened. Defaults to the time received.
    pub at: Option<ApiDateTime>,
    /// A retry with the same key returns the stored count; another body with it is a 409.
    #[validate(length(min = 1, max = 255))]
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportUsageResponse {
    /// `""` for the customer, else `<type code>=<key value>`.
    pub path: String,
    #[schema(value_type = String, format = "decimal")]
    pub count: Decimal,
    /// False when older than the last set, or a retry.
    pub applied: bool,
    pub replayed: bool,
}
