//! The shapes v2's entitlement reads and writes share (ADR-0013 §25, on ADR-0011's model):
//! values, policies, levels, rows and their owners, and the codes rows are read with.
use crate::AppState;
use crate::v2::Reader;
use crate::v2::bounds;
use crate::v2::customers::resources::resource_refs;
use crate::v2::ids;
use crate::v2::problem::{Problem, ProblemCode};
use crate::v2::rate_cards::{ActorKind, ChangeActor};
use crate::v2::refs::{
    FeatureRef, ProductRef, ReferenceIds, Refs, ResourceRef, ResourceTypeRef, SubscriptionRef,
};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, EntitlementEntityId, EntitlementId, FeatureId, ResourceId, ResourceTypeId,
    SubscriptionId, TenantId,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::entitlement_checks::LevelWindow;
use meteroid_store::domain::entitlement_fold::{
    Condition, Contribution, Effect, EvalLevel, GrantValue, LimitValue as DomainLimit,
    RowLevel as DomainRowLevel, Tier,
};
use meteroid_store::domain::entitlement_policy::{
    AtLimit as DomainAtLimit, FeaturePolicy, Period, Resets,
};
use meteroid_store::domain::entitlements::{
    Entitlement as DomainEntitlement, EntitlementSpec, Feature, FeatureKind as DomainKind,
    PeriodUnit, ValueType,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::domain::resources::{ResourceType, split_path};
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use utoipa::ToSchema;

// ── Values ──────────────────────────────────────────────────────────

/// The value a row gives, of the feature's type. A boolean takes `on` or `off`. A limit takes
/// `limit`, `unlimited` or `off`: `off` means not entitled at all, while a limit of 0 is
/// entitled to none. A value feature takes `number`, `text` (also a select's option) or `json`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntitlementValue {
    On(OnValue),
    Off(OffValue),
    Limit(LimitValue),
    Unlimited(UnlimitedValue),
    Number(NumberValue),
    Text(TextValue),
    Json(JsonValue),
}

/// A boolean feature switched on.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OnValue {}

/// Not entitled: a boolean switched off, or a limit not granted at all.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OffValue {}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LimitValue {
    /// Not negative.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal", example = "1000")]
    pub limit: Decimal,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UnlimitedValue {}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NumberValue {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal", example = "30")]
    pub number: Decimal,
}

/// A text, or one of a select's options.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextValue {
    pub text: String,
}

/// Any JSON value.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JsonValue {
    pub json: serde_json::Value,
}

impl From<&GrantValue> for EntitlementValue {
    fn from(value: &GrantValue) -> Self {
        match value {
            GrantValue::Boolean(true) => EntitlementValue::On(OnValue {}),
            GrantValue::Boolean(false) | GrantValue::Limit(DomainLimit::Off) => {
                EntitlementValue::Off(OffValue {})
            }
            GrantValue::Limit(DomainLimit::Amount(limit)) => EntitlementValue::Limit(LimitValue {
                limit: limit.normalize(),
            }),
            GrantValue::Limit(DomainLimit::Unlimited) => {
                EntitlementValue::Unlimited(UnlimitedValue {})
            }
            GrantValue::Number(number) => EntitlementValue::Number(NumberValue {
                number: number.normalize(),
            }),
            GrantValue::Text(text) => EntitlementValue::Text(TextValue { text: text.clone() }),
            GrantValue::Json(json) => EntitlementValue::Json(JsonValue { json: json.clone() }),
        }
    }
}

fn takes(kind: &DomainKind) -> &'static str {
    match kind {
        DomainKind::Boolean => "a boolean feature takes `on` or `off`",
        DomainKind::Limit(_) => "a limit takes `limit`, `unlimited` or `off`",
        DomainKind::Value(ValueType::Number { .. }) => "a number takes `number`",
        DomainKind::Value(ValueType::Text) => "a text takes `text`",
        DomainKind::Value(ValueType::Select { .. }) => "a select takes `text`, one of its options",
        DomainKind::Value(ValueType::Json) => "a json value takes `json`",
    }
}

/// The value as the feature's kind holds it, checked against it (a select's options, a limit
/// not negative).
pub fn grant_value(value: &EntitlementValue, kind: &DomainKind) -> Result<GrantValue, String> {
    let granted = match (value, kind) {
        (EntitlementValue::On(_), DomainKind::Boolean) => GrantValue::Boolean(true),
        (EntitlementValue::Off(_), DomainKind::Boolean) => GrantValue::Boolean(false),
        (EntitlementValue::Off(_), DomainKind::Limit(_)) => GrantValue::Limit(DomainLimit::Off),
        (EntitlementValue::Limit(LimitValue { limit }), DomainKind::Limit(_)) => {
            GrantValue::Limit(DomainLimit::Amount(*limit))
        }
        (EntitlementValue::Unlimited(_), DomainKind::Limit(_)) => {
            GrantValue::Limit(DomainLimit::Unlimited)
        }
        (
            EntitlementValue::Number(NumberValue { number }),
            DomainKind::Value(ValueType::Number { .. }),
        ) => GrantValue::Number(*number),
        (
            EntitlementValue::Text(TextValue { text }),
            DomainKind::Value(ValueType::Text | ValueType::Select { .. }),
        ) => GrantValue::Text(text.clone()),
        (EntitlementValue::Json(JsonValue { json }), DomainKind::Value(ValueType::Json)) => {
            GrantValue::Json(json.clone())
        }
        _ => return Err(takes(kind).to_owned()),
    };
    kind.check_value(&granted)?;
    Ok(granted)
}

// ── Policy ──────────────────────────────────────────────────────────

/// What reaching a limit does.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AtLimit {
    /// Usage past the limit (plus `grace_pct`) is refused.
    Deny,
    /// Usage goes on past the limit; a price on the same metric bills it.
    AllowAndBill,
    /// For a limit your app counts: what exists stays, nothing new is allowed.
    DenyNew,
}

/// How a limit behaves: when its usage resets and what reaching it does. The feature holds it;
/// a `set` row may give one of its own.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// When the usage counted against the limit starts again from zero. A limit your app counts
    /// never resets. `billing_cycle` follows the subscription; with none, each calendar month.
    pub resets: Reset,
    pub at_limit: AtLimit,
    /// `deny` only: how far past the limit usage is still allowed, in percent of the limit.
    #[serde(default)]
    pub grace_pct: u32,
    /// Percents of the limit at which an alert is sent, at most 10.
    #[serde(default)]
    pub thresholds: Vec<u32>,
}

fn unit(unit: PeriodUnit) -> Unit {
    match unit {
        PeriodUnit::Hour => Unit::Hour,
        PeriodUnit::Day => Unit::Day,
        PeriodUnit::Week => Unit::Week,
        PeriodUnit::Month => Unit::Month,
        PeriodUnit::Year => Unit::Year,
    }
}

fn period_unit(unit: Unit) -> PeriodUnit {
    match unit {
        Unit::Hour => PeriodUnit::Hour,
        Unit::Day => PeriodUnit::Day,
        Unit::Week => PeriodUnit::Week,
        Unit::Month => PeriodUnit::Month,
        Unit::Year => PeriodUnit::Year,
    }
}

impl From<&FeaturePolicy> for Policy {
    fn from(policy: &FeaturePolicy) -> Self {
        let window = |make: fn(Unit, u32) -> Reset| match policy.period {
            Some(Period { unit: u, interval }) => make(unit(u), interval),
            None => Reset::Never(NeverReset {}),
        };
        Policy {
            resets: match policy.resets {
                Resets::BillingCycle => Reset::BillingCycle(BillingCycleReset {}),
                Resets::Never => Reset::Never(NeverReset {}),
                Resets::Calendar => {
                    window(|unit, interval| Reset::Calendar(CalendarReset { unit, interval }))
                }
                Resets::Fixed => {
                    window(|unit, interval| Reset::FixedWindow(FixedWindowReset { unit, interval }))
                }
                Resets::Sliding => window(|unit, interval| {
                    Reset::SlidingWindow(SlidingWindowReset { unit, interval })
                }),
            },
            at_limit: match policy.at_limit {
                DomainAtLimit::Deny => AtLimit::Deny,
                DomainAtLimit::AllowAndBill => AtLimit::AllowAndBill,
                DomainAtLimit::DenyNew => AtLimit::DenyNew,
            },
            grace_pct: policy.grace_pct,
            thresholds: policy.thresholds.clone(),
        }
    }
}

impl From<&Policy> for FeaturePolicy {
    fn from(policy: &Policy) -> Self {
        let period = |u: Unit, interval: u32| {
            Some(Period {
                unit: period_unit(u),
                interval,
            })
        };
        let (resets, period) = match policy.resets {
            Reset::BillingCycle(_) => (Resets::BillingCycle, None),
            Reset::Never(_) => (Resets::Never, None),
            Reset::Calendar(w) => (Resets::Calendar, period(w.unit, w.interval)),
            Reset::FixedWindow(w) => (Resets::Fixed, period(w.unit, w.interval)),
            Reset::SlidingWindow(w) => (Resets::Sliding, period(w.unit, w.interval)),
        };
        let mut thresholds = policy.thresholds.clone();
        thresholds.sort_unstable();
        thresholds.dedup();
        FeaturePolicy {
            resets,
            period,
            at_limit: match policy.at_limit {
                AtLimit::Deny => DomainAtLimit::Deny,
                AtLimit::AllowAndBill => DomainAtLimit::AllowAndBill,
                AtLimit::DenyNew => DomainAtLimit::DenyNew,
            },
            grace_pct: policy.grace_pct,
            thresholds,
        }
    }
}

/// The policy a feature of `kind` would hold, refused at `pointer` when it cannot.
pub fn checked_policy(
    policy: &Policy,
    kind: &DomainKind,
    pointer: &str,
) -> Result<FeaturePolicy, Problem> {
    if !matches!(kind, DomainKind::Limit(_)) {
        return Err(Problem::field(
            pointer,
            "invalid",
            "Only a limit has a policy",
        ));
    }
    let policy = FeaturePolicy::from(policy);
    policy
        .validate(kind.counted_by_app())
        .map_err(|error| Problem::field(pointer, "invalid", error.to_string()))?;
    Ok(policy)
}

// ── Resets ──────────────────────────────────────────────────────────

/// When the usage counted against a limit starts again from zero.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reset {
    BillingCycle(BillingCycleReset),
    Never(NeverReset),
    Calendar(CalendarReset),
    FixedWindow(FixedWindowReset),
    SlidingWindow(SlidingWindowReset),
}

impl Default for Reset {
    /// A lifetime cap.
    fn default() -> Self {
        Reset::Never(NeverReset {})
    }
}

/// At each renewal of the subscription.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BillingCycleReset {}

/// A lifetime cap: the usage never starts again.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NeverReset {}

/// On calendar boundaries (the 1st of each month…), every `interval` calendar `unit`s.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarReset {
    pub unit: Unit,
    /// At least 1.
    #[schema(minimum = 1)]
    pub interval: u32,
}

/// Every `interval` `unit`s, counted from the subscription's start.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FixedWindowReset {
    pub unit: Unit,
    /// At least 1.
    #[schema(minimum = 1)]
    pub interval: u32,
}

/// The usage of the last `interval` `unit`s, up to now. Sends no alerts.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SlidingWindowReset {
    pub unit: Unit,
    /// At least 1.
    #[schema(minimum = 1)]
    pub interval: u32,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

// ── Levels, effects, owners ─────────────────────────────────────────

/// Where a value applies: for the customer as a whole (across all its resources), or for each
/// resource of `resource_type` (per site).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum LevelKind {
    Customer,
    ResourceType,
}

/// The resource a level is for: its type's code and its key value.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LevelResource {
    /// The resource type's code.
    #[serde(rename = "type")]
    pub resource_type: String,
    pub key: String,
}

/// The resource of a level the store names `site=s1`; none for the customer's (`""`).
pub fn level_resource(path: &str) -> Option<LevelResource> {
    split_path(path).map(|(code, key)| LevelResource {
        resource_type: code.to_owned(),
        key,
    })
}

/// How a row combines with the value of the owners before it: the feature's defaults, then the
/// plan and its version, the subscription, its add-ons and the customer's overrides, in that
/// order.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntitlementEffect {
    /// Replaces the value.
    Set,
    /// An add-on's: adds its value times the instances bought.
    Add,
    /// Raises the value to this one; never lowers it.
    RaiseTo,
    /// Lowers the value to this one; never raises it.
    CapAt,
}

impl From<Effect> for EntitlementEffect {
    fn from(effect: Effect) -> Self {
        match effect {
            Effect::Set => EntitlementEffect::Set,
            Effect::Add => EntitlementEffect::Add,
            Effect::RaiseTo => EntitlementEffect::RaiseTo,
            Effect::CapAt => EntitlementEffect::CapAt,
        }
    }
}

impl From<EntitlementEffect> for Effect {
    fn from(effect: EntitlementEffect) -> Self {
        match effect {
            EntitlementEffect::Set => Effect::Set,
            EntitlementEffect::Add => Effect::Add,
            EntitlementEffect::RaiseTo => Effect::RaiseTo,
            EntitlementEffect::CapAt => Effect::CapAt,
        }
    }
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OwnerType {
    /// The feature's defaults, which apply to every customer.
    Feature,
    Plan,
    PlanVersion,
    AddOn,
    Subscription,
    Quote,
    Customer,
}

/// What a row belongs to.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct EntitlementOwner {
    #[serde(rename = "type")]
    pub kind: OwnerType,
    pub id: String,
    /// The plan and version's, the add-on's or the customer's name, where known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

pub fn owner_of(entity: &EntitlementEntityId, name: Option<String>) -> EntitlementOwner {
    let (kind, id) = match entity {
        EntitlementEntityId::Feature(id) => (OwnerType::Feature, id.to_string()),
        EntitlementEntityId::Plan(id) => (OwnerType::Plan, id.to_string()),
        EntitlementEntityId::PlanVersion(id) => (OwnerType::PlanVersion, id.to_string()),
        EntitlementEntityId::AddOn(id) => (OwnerType::AddOn, id.to_string()),
        EntitlementEntityId::Subscription(id) => (OwnerType::Subscription, id.to_string()),
        EntitlementEntityId::Quote(id) => (OwnerType::Quote, id.to_string()),
        EntitlementEntityId::Customer(id) => (OwnerType::Customer, id.to_string()),
    };
    EntitlementOwner { kind, id, name }
}

/// The row the value comes from.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Origin {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub entitlement: EntitlementId,
    pub owner: EntitlementOwner,
    pub effect: EntitlementEffect,
}

impl From<&Contribution> for Origin {
    fn from(contribution: &Contribution) -> Self {
        Origin {
            entitlement: contribution.lineage_id,
            owner: owner_of(&contribution.source, contribution.source_name.clone()),
            effect: contribution.effect.into(),
        }
    }
}

/// The period a limit counted from events counts usage over.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UsageWindow {
    pub start: ApiDateTime,
    /// Exclusive: the next reset. Absent for a limit that never resets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<ApiDateTime>,
}

impl From<&LevelWindow> for UsageWindow {
    fn from(window: &LevelWindow) -> Self {
        UsageWindow {
            start: window.start.into(),
            end: window.end.map(Into::into),
        }
    }
}

/// The level and its resource type.
pub fn level_kind(level: EvalLevel, codes: &Codes) -> (LevelKind, Option<ResourceTypeRef>) {
    match level {
        EvalLevel::Customer => (LevelKind::Customer, None),
        EvalLevel::Type(id) => (LevelKind::ResourceType, Some(codes.type_ref(id))),
    }
}

// ── Features as readouts name them ──────────────────────────────────

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    Boolean,
    Value,
    Limit,
}

impl From<&DomainKind> for FeatureKind {
    fn from(kind: &DomainKind) -> Self {
        match kind {
            DomainKind::Boolean => FeatureKind::Boolean,
            DomainKind::Value(_) => FeatureKind::Value,
            DomainKind::Limit(_) => FeatureKind::Limit,
        }
    }
}

/// What a read says of its feature beside naming it (`FeatureRef`): its name, type, unit,
/// product and the resource type it applies per.
#[derive(Debug, Clone)]
pub struct FeatureFacts {
    pub feature: FeatureRef,
    pub name: String,
    pub kind: FeatureKind,
    pub unit: Option<String>,
    pub product: Option<ProductRef>,
    pub applies_per: Option<ResourceTypeRef>,
}

// ── Rows ────────────────────────────────────────────────────────────

/// A value given to a feature by its owner (a plan version, an add-on, a subscription, a
/// customer override or the feature's defaults). Editing a row keeps its id: the edit becomes a
/// new version, and `GET /entitlements/{id}/history` lists them all.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Entitlement {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: EntitlementId,
    /// The feature, by code.
    pub feature: String,
    pub value: EntitlementValue,
    pub effect: EntitlementEffect,
    /// Where the value applies. A row written without `level` reads at its feature's level
    /// (`applies_per`).
    pub level: LevelKind,
    /// With `level: resource_type`: the type's code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    /// A customer's override for one of its stored resources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceRef>,
    /// `{"<type>.<attribute>": [values]}`: the row applies only where each attribute holds one
    /// of its values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<BTreeMap<String, Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// A `set` row's own policy for its limit, in place of the feature's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
    pub owner: EntitlementOwner,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<ChangeActor>,
    /// When the row was first written.
    pub created_at: ApiDateTime,
    /// When this version was written.
    pub updated_at: ApiDateTime,
    /// When this version stopped applying: replaced by an edit, or revoked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<String>,
}

/// A row written on its owner. A row read back can be sent as is: its read-only fields are
/// ignored.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct EntitlementInput {
    /// The feature, by code or id.
    pub feature: String,
    pub value: EntitlementValue,
    /// Left out, the owner's usual one: `set` on a feature's defaults, a plan version and a
    /// subscription; `add` on an add-on bought in several instances, `raise_to` on another
    /// add-on and on a customer override.
    #[serde(default)]
    pub effect: Option<EntitlementEffect>,
    /// Left out, the feature's level: per resource of its `applies_per` type, else the customer.
    /// Set it for a value at another level, e.g. a cap across all sites on a feature per site.
    #[serde(default)]
    pub level: Option<LevelKind>,
    /// With `level: resource_type`: the type, by code or id.
    #[serde(default)]
    pub resource_type: Option<String>,
    #[serde(default)]
    pub condition: Option<BTreeMap<String, Vec<String>>>,
    #[serde(default)]
    pub valid_from: Option<ApiDateTime>,
    #[serde(default)]
    pub valid_until: Option<ApiDateTime>,
    #[serde(default)]
    pub reason: Option<String>,
    /// A limit's `set` row only.
    #[serde(default)]
    pub policy: Option<Policy>,
    /// Read-only: ignored.
    #[serde(default)]
    #[schema(read_only)]
    pub id: Option<EntitlementId>,
    /// Read-only: ignored.
    #[serde(default)]
    #[schema(read_only, value_type = Option<EntitlementOwner>)]
    pub owner: Option<serde_json::Value>,
    /// Read-only: ignored.
    #[serde(default)]
    #[schema(read_only, value_type = Option<ChangeActor>)]
    pub created_by: Option<serde_json::Value>,
    /// Read-only: ignored.
    #[serde(default)]
    #[schema(read_only)]
    pub created_at: Option<ApiDateTime>,
    /// Read-only: ignored.
    #[serde(default)]
    #[schema(read_only)]
    pub updated_at: Option<ApiDateTime>,
    /// Read-only: ignored.
    #[serde(default)]
    #[schema(read_only)]
    pub revoked_at: Option<ApiDateTime>,
    /// Read-only: ignored.
    #[serde(default)]
    #[schema(read_only, value_type = Option<String>)]
    pub revoked_by: Option<serde_json::Value>,
}

pub fn change_actor(actor: &Actor) -> ChangeActor {
    let (kind, id) = match actor {
        Actor::System => (ActorKind::System, None),
        Actor::User { id } => (ActorKind::User, Some(id.to_string())),
        Actor::ApiToken { id } => (ActorKind::ApiToken, Some(id.to_string())),
        Actor::Customer { id } => (ActorKind::Customer, Some(id.to_string())),
        Actor::QuoteRecipient { .. } => (ActorKind::QuoteRecipient, None),
        Actor::OAuthApp { id } => (ActorKind::OauthApp, Some(id.to_string())),
        Actor::Workflow { id } => (ActorKind::Workflow, Some(id.to_string())),
    };
    ChangeActor { kind, id }
}

/// A feature as its rows read: its code, and the level a row without one applies at.
#[derive(Debug, Clone)]
pub struct FeatureKey {
    pub code: String,
    pub applies_per: Option<ResourceTypeId>,
}

impl From<&Feature> for FeatureKey {
    fn from(feature: &Feature) -> Self {
        FeatureKey {
            code: feature.code.clone(),
            applies_per: feature.level_type_id,
        }
    }
}

/// The keys rows and reads are read with: features' codes, resource types', stored resources,
/// and the products and subscriptions reads name (`refs`), each read once per response.
#[derive(Debug, Default, Clone)]
pub struct Codes {
    pub features: HashMap<FeatureId, FeatureKey>,
    pub types: HashMap<ResourceTypeId, String>,
    pub resources: HashMap<ResourceId, ResourceRef>,
    pub refs: Refs,
}

impl Codes {
    pub async fn types(state: &impl Reader, tenant_id: TenantId) -> Result<Self, Problem> {
        let types = state.store().list_resource_types(tenant_id, true).await?;
        Ok(Codes {
            types: types.into_iter().map(|t| (t.id, t.code)).collect(),
            ..Codes::default()
        })
    }

    /// The products of `features` and the `subscriptions` a read names, with the tenant's
    /// resource type codes.
    pub async fn of_features<'a>(
        state: &impl Reader,
        tenant_id: TenantId,
        features: impl IntoIterator<Item = &'a Feature>,
        subscriptions: impl IntoIterator<Item = SubscriptionId>,
    ) -> Result<Self, Problem> {
        let mut codes = Codes::types(state, tenant_id).await?;
        let ids = ReferenceIds {
            products: features
                .into_iter()
                .filter_map(|f| f.product.as_ref().map(|p| p.id))
                .collect(),
            subscriptions: subscriptions.into_iter().collect(),
            ..ReferenceIds::default()
        };
        codes.refs = Refs::load(state.store(), tenant_id, &ids).await?;
        Ok(codes)
    }

    /// The feature codes, resource type codes and resources of `rows`.
    pub async fn of_rows(
        state: &impl Reader,
        tenant_id: TenantId,
        rows: &[DomainEntitlement],
    ) -> Result<Self, Problem> {
        let mut codes = Codes::types(state, tenant_id).await?;
        let feature_ids: Vec<FeatureId> = rows
            .iter()
            .map(|r| r.feature_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        codes.features = state
            .store()
            .list_features_by_ids(&feature_ids, tenant_id)
            .await?
            .iter()
            .map(|f| (f.id, FeatureKey::from(f)))
            .collect();
        // Archived features are left out of the list: read one by one.
        for id in feature_ids {
            if let std::collections::hash_map::Entry::Vacant(slot) = codes.features.entry(id) {
                slot.insert(FeatureKey::from(
                    &state.store().get_feature(id, tenant_id).await?,
                ));
            }
        }
        codes.resources =
            resource_refs(state, tenant_id, rows.iter().filter_map(|r| r.resource_id)).await?;
        Ok(codes)
    }

    pub fn type_code(&self, id: ResourceTypeId) -> String {
        self.types
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    pub fn type_ref(&self, id: ResourceTypeId) -> ResourceTypeRef {
        ResourceTypeRef {
            id,
            code: self.type_code(id),
        }
    }

    pub fn subscription(&self, id: SubscriptionId) -> SubscriptionRef {
        self.refs.subscription(id)
    }

    pub fn facts(&self, feature: &Feature) -> FeatureFacts {
        FeatureFacts {
            feature: FeatureRef {
                id: feature.id,
                code: feature.code.clone(),
            },
            name: feature.name.clone(),
            kind: (&feature.kind).into(),
            unit: feature.unit.clone(),
            product: feature.product.as_ref().map(|p| self.refs.product(p.id)),
            applies_per: feature.level_type_id.map(|t| self.type_ref(t)),
        }
    }

    pub fn entitlement(&self, row: DomainEntitlement) -> Entitlement {
        let feature = self.features.get(&row.feature_id);
        let (level, resource_type) = match row.level.eval(feature.and_then(|f| f.applies_per)) {
            EvalLevel::Customer => (LevelKind::Customer, None),
            EvalLevel::Type(id) => (LevelKind::ResourceType, Some(self.type_code(id))),
        };
        Entitlement {
            id: row.id,
            feature: feature.map_or_else(|| row.feature_id.to_string(), |f| f.code.clone()),
            value: (&row.value).into(),
            effect: row.effect.into(),
            level,
            resource_type,
            // A resource an override names is never deleted (`DELETE` refuses it).
            resource: row.resource_id.map(|id| {
                self.resources
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| ResourceRef::of_id(id))
            }),
            condition: row.condition.map(|c| c.0),
            valid_from: row.valid_from.map(Into::into),
            valid_until: row.valid_until.map(Into::into),
            reason: row.reason,
            policy: row.policy_override.as_ref().map(Into::into),
            owner: owner_of(&row.entity, None),
            created_by: row.created_by.as_ref().map(change_actor),
            created_at: row.lineage_created_at.into(),
            updated_at: row.created_at.into(),
            revoked_at: row.revoked_at.map(Into::into),
            revoked_by: row.revoked_by.map(|id| id.to_string()),
        }
    }
}

/// The feature a row names, at `pointer`: not found, or archived, is refused there.
pub async fn granted_feature(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
    pointer: &str,
) -> Result<Feature, Problem> {
    let feature = ids::feature(state, tenant_id, raw)
        .await
        .map_err(|problem| match problem.code {
            ProblemCode::NotFound => {
                Problem::field(pointer, "not_found", format!("No feature '{}'", raw.trim()))
            }
            _ => problem,
        })?;
    if feature.status == FeatureStatusEnum::Archived {
        return Err(Problem::field(
            pointer,
            "archived",
            format!("The feature '{}' is archived", feature.code),
        ));
    }
    Ok(feature)
}

/// What a row's input asks, checked against its feature and its owner's tier at `pointer` (a
/// JSON pointer to the row): a value of the feature's kind, an effect the tier takes, a level
/// and a policy the feature can hold. The store checks conditions and overlaps.
pub fn spec_of(
    input: &EntitlementInput,
    feature: &Feature,
    types: &[ResourceType],
    tier: Tier,
    multi_instance: bool,
    pointer: &str,
) -> Result<EntitlementSpec, Problem> {
    let at = |field: &str| format!("{pointer}/{field}");
    let value = grant_value(&input.value, &feature.kind)
        .map_err(|message| Problem::field(at("value"), "invalid", message))?;

    let level = match (input.level, input.resource_type.as_deref()) {
        (Some(LevelKind::ResourceType), Some(raw)) => {
            let raw = raw.trim();
            let resource_type = types
                .iter()
                .find(|t| {
                    t.code == raw || t.id.to_string() == raw || t.id.as_uuid().to_string() == raw
                })
                .ok_or_else(|| {
                    Problem::field(
                        at("resource_type"),
                        "not_found",
                        format!("No resource type '{raw}'"),
                    )
                })?;
            if resource_type.archived_at.is_some() {
                return Err(Problem::field(
                    at("resource_type"),
                    "archived",
                    format!("The resource type '{}' is archived", resource_type.code),
                ));
            }
            DomainRowLevel::Type(resource_type.id)
        }
        (Some(LevelKind::ResourceType), None) => {
            return Err(Problem::field(
                at("resource_type"),
                "required",
                "A row at `level: resource_type` names its resource type",
            ));
        }
        (_, Some(_)) => {
            return Err(Problem::field(
                at("resource_type"),
                "invalid",
                "Only a row at `level: resource_type` names a resource type",
            ));
        }
        (None, None) => DomainRowLevel::Feature,
        (Some(LevelKind::Customer), None) => DomainRowLevel::Customer,
    }
    .normalised(feature.level_type_id);

    let order = feature.value_order();
    if order.is_unordered() && level != DomainRowLevel::Feature {
        return Err(Problem::field(
            at("level"),
            "invalid",
            "A text, json or unordered select value applies at its feature's level only: leave \
             `level` out",
        ));
    }
    let effect = input
        .effect
        .map(Effect::from)
        .unwrap_or_else(|| tier.default_effect(&order, multi_instance));
    let allowed = tier.allowed_effects(&order);
    if !allowed.contains(&effect) {
        let names: Vec<&str> = allowed
            .iter()
            .map(|e| match e {
                Effect::Set => "set",
                Effect::Add => "add",
                Effect::RaiseTo => "raise_to",
                Effect::CapAt => "cap_at",
            })
            .collect();
        return Err(Problem::field(
            at("effect"),
            "invalid",
            format!("This owner's rows take {}", names.join(", ")),
        ));
    }
    let policy_override = match &input.policy {
        None => None,
        Some(_) if effect != Effect::Set => {
            return Err(Problem::field(
                at("policy"),
                "invalid",
                "Only a `set` row overrides its limit's policy",
            ));
        }
        Some(policy) => Some(checked_policy(policy, &feature.kind, &at("policy"))?),
    };
    if let (Some(from), Some(until)) = (input.valid_from, input.valid_until)
        && until <= from
    {
        return Err(Problem::field(
            at("valid_until"),
            "invalid",
            "valid_until is after valid_from",
        ));
    }
    bounds::description(input.reason.as_deref(), &at("reason"))?;
    Ok(EntitlementSpec {
        feature_id: feature.id,
        value,
        // Left out, the store gives the tier's default: it knows how an add-on is bought.
        effect: input.effect.map(Effect::from),
        level,
        condition: input
            .condition
            .clone()
            .filter(|c| !c.is_empty())
            .map(Condition),
        valid_from: input.valid_from.map(Into::into),
        valid_until: input.valid_until.map(Into::into),
        reason: input
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned),
        policy_override,
    })
}
