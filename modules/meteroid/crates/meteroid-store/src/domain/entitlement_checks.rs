//! What resolution, the check (ADR 0011 §3.11, §3.14) and the snapshot answer.
use crate::domain::entitlement_fold::{Contribution, EvalLevel, GrantValue};
use crate::domain::entitlement_policy::FeaturePolicy;
use crate::domain::entitlements::{
    EffectiveEntitlement, EffectiveEntitlementValue, Feature, ResetPeriod,
};
use crate::domain::resources::{Attributes, ResolvedResource};
use chrono::{DateTime, Utc};
use common_domain::ids::{ApiTokenId, FeatureId, ResourceId, SubscriptionId};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use uuid::Uuid;

pub use crate::domain::entitlement_levels::{
    CheckReason, LevelUsage, LevelValue, ReasonType, Uncountable,
};

/// Whether a feature's values hold for the whole customer, or only per resource: only resource
/// subscriptions include it, so its values are not OR-ed into a customer-wide one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureScope {
    Customer,
    Resource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelWindow {
    pub reset_period: ResetPeriod,
    pub start: DateTime<Utc>,
    /// Exclusive; `None` for a window that never ends.
    pub end: Option<DateTime<Utc>>,
}

/// One step of the deciding-subscription walk (§3.7): a subscription including the feature at
/// the level, with the value its fold gives there.
#[derive(Debug, Clone, PartialEq)]
pub struct DecidingCandidate {
    pub subscription_id: SubscriptionId,
    /// For a resource, its path; `None` for the whole customer.
    pub resource_path: Option<String>,
    pub value: Option<GrantValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLevel {
    pub level: EvalLevel,
    /// The level that answers when nothing else does: no row there means not entitled.
    pub own: bool,
    /// `''` for the customer, `site=s1` for a resource.
    pub path: String,
    pub value: LevelValue,
    pub policy: FeaturePolicy,
    /// The Set to row whose policy override is in effect.
    pub policy_from: Option<common_domain::ids::EntitlementId>,
    pub deciding_subscription: Option<SubscriptionId>,
    /// Customer subscriptions answered a check without a resource while resource ones also
    /// include the feature.
    pub resource_omitted: bool,
    /// Limits counted from events; `None` for the others.
    pub window: Option<LevelWindow>,
    /// The next `valid_from` or `valid_until` ahead among the level's rows.
    pub next_change: Option<DateTime<Utc>>,
    /// The contributions of the deciding fold, discarded rows included.
    pub explain: Vec<Contribution>,
    pub candidates: Vec<DecidingCandidate>,
}

#[derive(Debug, Clone)]
pub struct ResolvedFeature {
    pub feature: Feature,
    /// The feature's own level first.
    pub levels: Vec<ResolvedLevel>,
    pub scope: FeatureScope,
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedCustomer {
    pub features: Vec<ResolvedFeature>,
}

/// The warning of a check or a snapshot holding a level whose usage could not be read.
pub const USAGE_UNAVAILABLE_WARNING: &str = "usage_unavailable";

/// The warnings of snapshot features: [`USAGE_UNAVAILABLE_WARNING`] when a level's usage could
/// not be read.
pub fn usage_warnings<'a>(features: impl IntoIterator<Item = &'a SnapshotFeature>) -> Vec<String> {
    features
        .into_iter()
        .flat_map(|f| &f.levels)
        .any(|l| l.usage_unavailable)
        .then(|| USAGE_UNAVAILABLE_WARNING.to_owned())
        .into_iter()
        .collect()
}

/// The warnings of effective entitlements: [`USAGE_UNAVAILABLE_WARNING`] when metering could not
/// answer for one's usage.
pub fn effective_usage_warnings(entitlements: &[EffectiveEntitlement]) -> Vec<String> {
    entitlements
        .iter()
        .any(|e| {
            matches!(&e.value, EffectiveEntitlementValue::Metered { usage, .. } if usage.unavailable)
        })
        .then(|| USAGE_UNAVAILABLE_WARNING.to_owned())
        .into_iter()
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    /// Read at check time from metering and the app counts (core).
    Read,
    /// From the live engine's level monitors (Enterprise).
    Monitor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, strum::IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CheckHint {
    ResourceOmitted,
}

#[derive(Debug, Clone, Default)]
pub struct CheckRequest {
    /// At most one key, of any resource type (`{"site_id": "s1"}`).
    pub resources: BTreeMap<String, String>,
    pub attributes: Attributes,
    /// The units the caller is about to use; 1 when absent.
    pub requested: Option<Decimal>,
    /// Server key only: the app's own count, for a limit counted by the app.
    pub current_count: Option<Decimal>,
    /// Server key only.
    pub explain: bool,
    /// Accepted and ignored (§3.7).
    pub subscription_id: Option<SubscriptionId>,
    /// The key the call came with, for the asserted-attribute log line.
    pub api_key_id: Option<ApiTokenId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckedFeature {
    pub id: FeatureId,
    pub code: String,
}

/// The exact count of a path, the last set and the adds since (§3.11).
#[derive(Debug, Clone, PartialEq)]
pub struct AppCountExplain {
    pub path: String,
    pub count: Decimal,
    pub last_set_value: Option<Decimal>,
    pub last_set_at: Option<DateTime<Utc>>,
    pub last_set_key: Option<String>,
    pub adds_since: i64,
    pub adds_sum: Decimal,
    pub last_add_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckLevelExplain {
    pub contributions: Vec<Contribution>,
    pub candidates: Vec<DecidingCandidate>,
    pub app_counts: Vec<AppCountExplain>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckLevel {
    pub level: EvalLevel,
    pub path: String,
    pub value: Option<GrantValue>,
    /// The level's limit when it is a number.
    pub limit: Option<Decimal>,
    pub used: Option<Decimal>,
    pub near_limit: bool,
    pub reset_at: Option<DateTime<Utc>>,
    /// A limit counted from events: the window its usage is counted over.
    pub window: Option<LevelWindow>,
    pub deciding_subscription: Option<SubscriptionId>,
    pub denial: Option<CheckReason>,
    /// Past the limit of a level that allows and bills.
    pub in_overage: bool,
    pub explain: Option<CheckLevelExplain>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckResponse {
    pub feature: CheckedFeature,
    pub allowed: bool,
    pub reason: Option<CheckReason>,
    pub reason_type: Option<ReasonType>,
    /// The own level's value, capped by the others in the feature's order.
    pub value: Option<GrantValue>,
    pub levels: Vec<CheckLevel>,
    pub near_limit: bool,
    pub retry_after: Option<DateTime<Utc>>,
    pub hint: Option<CheckHint>,
    pub attributes_ignored: Vec<String>,
    pub warnings: Vec<String>,
    pub usage_source: UsageSource,
    /// When the oldest usage answering was computed, if older than the check: a monitor's last
    /// reading, kept while metering is unavailable.
    pub usage_as_of: Option<DateTime<Utc>>,
}

/// A level of the check as the live engine reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveCheckLevel {
    pub level: EvalLevel,
    pub path: String,
    /// `level_monitor_id(feature, customer, level)`.
    pub monitor_id: Uuid,
    pub limit: Option<Decimal>,
    pub policy: FeaturePolicy,
    pub window: Option<LevelWindow>,
    pub deciding_subscription: Option<SubscriptionId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveCheckRequest {
    pub feature_id: FeatureId,
    pub requested: Decimal,
    /// Limits counted from events.
    pub levels: Vec<LiveCheckLevel>,
    /// The own level's: budgets and credit at zero follow it for a feature without a metric.
    pub deciding_subscription: Option<SubscriptionId>,
    /// The check's resource: budgets and credit at zero follow the subscription billing its
    /// events (§3.7), never one the request names.
    pub resource: Option<ResolvedResource>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveLevelUsage {
    pub level: EvalLevel,
    pub used: Decimal,
    /// When the usage was computed.
    pub as_of: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveCheck {
    /// Levels the monitors hold a current reading of; the others keep the read usage.
    pub levels: Vec<LiveLevelUsage>,
    /// The others' last reading, whatever its age: their usage when metering cannot answer.
    pub last: Vec<LiveLevelUsage>,
    /// `BUDGET_EXCEEDED` or `CREDIT_EXHAUSTED`.
    pub denial: Option<CheckReason>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotLevel {
    pub level: EvalLevel,
    pub path: String,
    pub value: LevelValue,
    pub policy: FeaturePolicy,
    pub deciding_subscription: Option<SubscriptionId>,
    pub window: Option<LevelWindow>,
    pub used: Option<Decimal>,
    /// Metering could not answer for `used`.
    pub usage_unavailable: bool,
    /// The row that set the value, named; `None` without a value.
    pub origin: Option<Contribution>,
}

#[derive(Debug, Clone)]
pub struct SnapshotFeature {
    pub feature: Feature,
    pub scope: FeatureScope,
    pub value: Option<GrantValue>,
    pub levels: Vec<SnapshotLevel>,
}

#[derive(Debug, Clone)]
pub struct ResourceSnapshot {
    pub resource_id: ResourceId,
    pub features: Vec<SnapshotFeature>,
    /// [`USAGE_UNAVAILABLE_WARNING`] when a level's usage could not be read.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub etag: String,
    pub features: Vec<SnapshotFeature>,
    pub attributes_ignored: Vec<String>,
    /// [`USAGE_UNAVAILABLE_WARNING`] when a level's usage could not be read.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum SnapshotRead {
    Modified(Snapshot),
    /// `If-None-Match` matched the etag: 304.
    NotModified {
        etag: String,
    },
}
