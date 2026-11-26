use crate::domain::entitlement_fold::{
    Condition, Effect, EvalLevel, FoldRow, GrantValue, LimitValue, RowLevel, Tier, ValueOrder,
};
use crate::domain::entitlement_policy::{AtLimit, FeaturePolicy, Resets};
use crate::domain::enums::FeatureStatusEnum;
use crate::domain::resources::Attributes;
use crate::errors::{StoreError, StoreErrorReport};
use chrono::{DateTime, Utc};
use common_domain::actor::{Actor, ActorType};
use common_domain::ids::{
    BaseId, BillableMetricId, EntitlementEntityId, EntitlementId, FeatureId, ProductId, ResourceId,
    ResourceTypeId, SubscriptionId, TenantId,
};
use diesel_models::entitlements::{EntitlementRow, FeatureRowNew, FeatureWithProductRow};
use diesel_models::enums::{
    ConfigValueTypeEnum as DbConfigValueTypeEnum, EntitlementEffectEnum, EntitlementLevelEnum,
    FeatureKindEnum, FeatureLimitCountedByEnum, FeatureStatusEnum as DbFeatureStatusEnum,
    FeatureTypeEnum as DbFeatureTypeEnum, FeatureValueTypeEnum,
};
use error_stack::Report;
use rust_decimal::Decimal;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodUnit {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

/// When the consumed counter resets.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResetPeriod {
    BillingCycle,
    Calendar { unit: PeriodUnit, interval: u32 },
    FixedWindow { unit: PeriodUnit, interval: u32 },
    SlidingWindow { unit: PeriodUnit, interval: u32 },
    Never,
}

/// What happens past a limit: refused once the limit plus its grace is reached, or allowed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OverageBehavior {
    Block {
        #[serde(skip_serializing_if = "Option::is_none")]
        grace_period_pct: Option<u32>,
    },
    Allow,
}

#[derive(Clone, Debug)]
pub struct EntitlementUsage {
    pub consumed: Option<Decimal>,
    pub remaining: Option<Decimal>,
    pub period_start: Option<DateTime<Utc>>,
    pub reset_at: Option<DateTime<Utc>>,
    /// Metering could not answer for `consumed`, which is absent.
    pub unavailable: bool,
}

/// A value feature's value as the legacy readout shows it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ConfigValue {
    Number(Decimal),
    Boolean(bool),
    Text(String),
    Json(serde_json::Value),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValueType {
    Number { higher_is_better: bool },
    Text,
    Select { options: Vec<String>, ordered: bool },
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitCountedBy {
    Events(BillableMetricId),
    App,
}

/// ADR 0011 §3.1.
#[derive(Clone, Debug, PartialEq)]
pub enum FeatureKind {
    Boolean,
    Value(ValueType),
    Limit(LimitCountedBy),
}

impl FeatureKind {
    pub fn value_order(&self) -> ValueOrder {
        match self {
            FeatureKind::Boolean => ValueOrder::Boolean,
            FeatureKind::Limit(_) => ValueOrder::Limit,
            FeatureKind::Value(ValueType::Number { higher_is_better }) => ValueOrder::Number {
                higher_is_better: *higher_is_better,
            },
            FeatureKind::Value(ValueType::Select {
                options,
                ordered: true,
            }) => ValueOrder::Ranked(options.clone()),
            FeatureKind::Value(_) => ValueOrder::Unordered,
        }
    }

    pub fn metric_id(&self) -> Option<BillableMetricId> {
        match self {
            FeatureKind::Limit(LimitCountedBy::Events(metric_id)) => Some(*metric_id),
            _ => None,
        }
    }

    pub fn counted_by_app(&self) -> bool {
        matches!(self, FeatureKind::Limit(LimitCountedBy::App))
    }

    /// The policy a limit starts with: app counts never reset and deny new.
    pub fn default_policy(&self) -> Option<FeaturePolicy> {
        match self {
            FeatureKind::Limit(LimitCountedBy::App) => Some(FeaturePolicy {
                resets: Resets::Never,
                at_limit: AtLimit::DenyNew,
                ..FeaturePolicy::default()
            }),
            FeatureKind::Limit(LimitCountedBy::Events(_)) => Some(FeaturePolicy::default()),
            _ => None,
        }
    }

    /// Whether `value` is one of this kind's values.
    pub fn check_value(&self, value: &GrantValue) -> Result<(), String> {
        let fits = match (self, value) {
            (FeatureKind::Boolean, GrantValue::Boolean(_)) => true,
            (FeatureKind::Limit(_), GrantValue::Limit(LimitValue::Amount(n)))
                if n.is_sign_negative() =>
            {
                return Err("a limit must not be negative".into());
            }
            (FeatureKind::Limit(_), GrantValue::Limit(_)) => true,
            (FeatureKind::Value(ValueType::Number { .. }), GrantValue::Number(_)) => true,
            (FeatureKind::Value(ValueType::Text), GrantValue::Text(_)) => true,
            (FeatureKind::Value(ValueType::Json), GrantValue::Json(_)) => true,
            (FeatureKind::Value(ValueType::Select { options, .. }), GrantValue::Text(t)) => {
                if !options.contains(t) {
                    return Err(format!("`{t}` is not one of the feature's options"));
                }
                true
            }
            _ => false,
        };
        fits.then_some(())
            .ok_or_else(|| "the value does not match the feature's kind".into())
    }

    /// `feature.feature_type` and the config columns, written until phase 6.
    fn legacy_columns(
        &self,
    ) -> (
        DbFeatureTypeEnum,
        Option<Vec<Option<String>>>,
        Option<DbConfigValueTypeEnum>,
    ) {
        match self {
            FeatureKind::Boolean => (DbFeatureTypeEnum::Boolean, None, None),
            FeatureKind::Limit(_) => (DbFeatureTypeEnum::Metered, None, None),
            FeatureKind::Value(value_type) => {
                let (config, options) = match value_type {
                    ValueType::Number { .. } => (DbConfigValueTypeEnum::Number, None),
                    ValueType::Text => (DbConfigValueTypeEnum::Text, None),
                    ValueType::Json => (DbConfigValueTypeEnum::Json, None),
                    ValueType::Select { options, .. } => (
                        DbConfigValueTypeEnum::Select,
                        Some(options.iter().cloned().map(Some).collect()),
                    ),
                };
                (DbFeatureTypeEnum::Config, options, Some(config))
            }
        }
    }

    /// The `kind`, `value_type`, `number_higher_is_better`, `select_options`, `select_ordered`
    /// and `limit_counted_by` columns.
    #[allow(clippy::type_complexity)]
    fn columns(
        &self,
    ) -> (
        FeatureKindEnum,
        Option<FeatureValueTypeEnum>,
        bool,
        Option<serde_json::Value>,
        bool,
        Option<FeatureLimitCountedByEnum>,
    ) {
        let value = |t, higher, options: Option<&Vec<String>>, ordered| {
            (
                FeatureKindEnum::Value,
                Some(t),
                higher,
                options.map(|o| serde_json::json!(o)),
                ordered,
                None,
            )
        };
        match self {
            FeatureKind::Boolean => (FeatureKindEnum::Boolean, None, true, None, false, None),
            FeatureKind::Limit(counted_by) => (
                FeatureKindEnum::Limit,
                None,
                true,
                None,
                false,
                Some(match counted_by {
                    LimitCountedBy::Events(_) => FeatureLimitCountedByEnum::Events,
                    LimitCountedBy::App => FeatureLimitCountedByEnum::App,
                }),
            ),
            FeatureKind::Value(ValueType::Number { higher_is_better }) => {
                value(FeatureValueTypeEnum::Number, *higher_is_better, None, false)
            }
            FeatureKind::Value(ValueType::Text) => {
                value(FeatureValueTypeEnum::Text, true, None, false)
            }
            FeatureKind::Value(ValueType::Json) => {
                value(FeatureValueTypeEnum::Json, true, None, false)
            }
            FeatureKind::Value(ValueType::Select { options, ordered }) => {
                value(FeatureValueTypeEnum::Select, true, Some(options), *ordered)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FeatureListOrder {
    #[default]
    Name,
    /// Grouped by product name, global features last.
    Product,
}

impl From<FeatureListOrder> for diesel_models::entitlements::FeatureListOrder {
    fn from(order: FeatureListOrder) -> Self {
        match order {
            FeatureListOrder::Name => Self::Name,
            FeatureListOrder::Product => Self::Product,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Feature {
    pub id: FeatureId,
    pub tenant_id: TenantId,
    /// `None` for tenant-global features.
    pub product: Option<FeatureProductRef>,
    pub name: String,
    /// The immutable key code references the feature by.
    pub code: String,
    pub description: Option<String>,
    pub kind: FeatureKind,
    pub unit: Option<String>,
    /// The feature's own level (§3.3); `None` is the customer.
    pub level_type_id: Option<ResourceTypeId>,
    /// Limits only.
    pub policy: Option<FeaturePolicy>,
    pub status: FeatureStatusEnum,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The open rows of the feature's default tier.
    pub defaults: Vec<Entitlement>,
}

impl Feature {
    pub fn value_order(&self) -> ValueOrder {
        self.kind.value_order()
    }

    pub fn own_level(&self) -> EvalLevel {
        EvalLevel::of_feature(self.level_type_id)
    }

    pub fn policy_or_default(&self) -> FeaturePolicy {
        self.policy
            .clone()
            .or_else(|| self.kind.default_policy())
            .unwrap_or_default()
    }

    pub fn to_ref(&self) -> FeatureRef {
        FeatureRef {
            id: self.id,
            name: self.name.clone(),
            code: self.code.clone(),
            product: self.product.clone(),
        }
    }
}

fn invalid(message: impl Into<String>) -> StoreErrorReport {
    Report::new(StoreError::InvalidArgument(message.into()))
}

pub(crate) fn from_json<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
    what: &str,
) -> Result<T, StoreErrorReport> {
    serde_json::from_value(value).map_err(|e| invalid(format!("invalid {what}: {e}")))
}

pub(crate) fn to_json<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).expect("entitlement values serialise")
}

impl TryFrom<FeatureWithProductRow> for Feature {
    type Error = StoreErrorReport;

    fn try_from(row: FeatureWithProductRow) -> Result<Self, Self::Error> {
        let FeatureWithProductRow { feature, product } = row;
        let kind = match feature.kind {
            FeatureKindEnum::Boolean => FeatureKind::Boolean,
            FeatureKindEnum::Limit => FeatureKind::Limit(match feature.limit_counted_by {
                Some(FeatureLimitCountedByEnum::App) => LimitCountedBy::App,
                _ => LimitCountedBy::Events(
                    feature
                        .metric_id
                        .ok_or_else(|| invalid("a limit counted from events needs a metric"))?,
                ),
            }),
            FeatureKindEnum::Value => FeatureKind::Value(match feature.value_type {
                Some(FeatureValueTypeEnum::Number) => ValueType::Number {
                    higher_is_better: feature.number_higher_is_better,
                },
                Some(FeatureValueTypeEnum::Json) => ValueType::Json,
                Some(FeatureValueTypeEnum::Select) => ValueType::Select {
                    options: feature
                        .select_options
                        .map(|o| from_json(o, "select options"))
                        .transpose()?
                        .unwrap_or_default(),
                    ordered: feature.select_ordered,
                },
                Some(FeatureValueTypeEnum::Text) | None => ValueType::Text,
            }),
        };
        Ok(Feature {
            id: feature.id,
            tenant_id: feature.tenant_id,
            product: product.map(|p| FeatureProductRef {
                id: p.id,
                name: p.name,
            }),
            name: feature.name,
            code: feature.code,
            description: feature.description,
            kind,
            unit: feature.unit,
            level_type_id: feature.level_resource_type_id,
            policy: feature
                .policy
                .map(|p| from_json(p, "feature policy"))
                .transpose()?,
            status: feature.status.into(),
            created_at: feature.created_at,
            updated_at: feature.updated_at,
            defaults: vec![],
        })
    }
}

#[derive(Clone, Debug)]
pub struct FeatureNew {
    pub tenant_id: TenantId,
    pub product_id: Option<ProductId>,
    pub name: String,
    pub code: String,
    pub description: Option<String>,
    pub kind: FeatureKind,
    pub unit: Option<String>,
    pub level_type_id: Option<ResourceTypeId>,
    /// Limits only; `None` takes the kind's default.
    pub policy: Option<FeaturePolicy>,
    /// A Set to row on the default tier, at the feature's own level.
    pub default: Option<GrantValue>,
}

impl From<FeatureNew> for FeatureRowNew {
    fn from(f: FeatureNew) -> Self {
        let (feature_type, config_options, config_value_type) = f.kind.legacy_columns();
        let (kind, value_type, number_higher_is_better, select_options, select_ordered, counted_by) =
            f.kind.columns();
        FeatureRowNew {
            id: FeatureId::new(),
            tenant_id: f.tenant_id,
            product_id: f.product_id,
            name: f.name,
            code: f.code,
            description: f.description,
            feature_type,
            status: DbFeatureStatusEnum::Active,
            metric_id: f.kind.metric_id(),
            config_options,
            config_value_type,
            kind,
            value_type,
            unit: f.unit,
            number_higher_is_better,
            select_options,
            select_ordered,
            limit_counted_by: counted_by,
            level_resource_type_id: f.level_type_id,
            policy: f
                .policy
                .or_else(|| f.kind.default_policy())
                .map(|p| to_json(&p)),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FeatureUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub product_id: Option<Option<ProductId>>,
    pub unit: Option<Option<String>>,
    /// Numbers only.
    pub number_higher_is_better: Option<bool>,
    /// Selects only; an option still used by a row can't be removed.
    pub select_options: Option<Vec<String>>,
    pub select_ordered: Option<bool>,
    /// Moves the rows that follow the feature's level (§3.3).
    pub level_type_id: Option<Option<ResourceTypeId>>,
    /// Limits only.
    pub policy: Option<FeaturePolicy>,
}

/// A row (§3.2), addressed by its lineage. Edits revoke it and insert a successor.
#[derive(Clone, Debug, PartialEq)]
pub struct Entitlement {
    /// The lineage id, which the API addresses rows by.
    pub id: EntitlementId,
    /// This version's own id.
    pub version_id: EntitlementId,
    pub tenant_id: TenantId,
    pub feature_id: FeatureId,
    pub entity: EntitlementEntityId,
    pub effect: Effect,
    pub level: RowLevel,
    /// A customer row on one stored resource: a resource override.
    pub resource_id: Option<ResourceId>,
    pub value: GrantValue,
    pub condition: Option<Condition>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub policy_override: Option<FeaturePolicy>,
    pub created_by: Option<Actor>,
    pub revoked_by: Option<Uuid>,
    pub lineage_created_at: DateTime<Utc>,
    /// Where a window without an activation starts: the earlier of the lineage's first
    /// `valid_from` and its creation, which no edit moves.
    pub lineage_anchor: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<Effect> for EntitlementEffectEnum {
    fn from(effect: Effect) -> Self {
        match effect {
            Effect::Set => EntitlementEffectEnum::Set,
            Effect::Add => EntitlementEffectEnum::Add,
            Effect::RaiseTo => EntitlementEffectEnum::RaiseTo,
            Effect::CapAt => EntitlementEffectEnum::CapAt,
        }
    }
}

impl From<EntitlementEffectEnum> for Effect {
    fn from(effect: EntitlementEffectEnum) -> Self {
        match effect {
            EntitlementEffectEnum::Set => Effect::Set,
            EntitlementEffectEnum::Add => Effect::Add,
            EntitlementEffectEnum::RaiseTo => Effect::RaiseTo,
            EntitlementEffectEnum::CapAt => Effect::CapAt,
        }
    }
}

impl RowLevel {
    pub fn from_columns(level: EntitlementLevelEnum, type_id: Option<ResourceTypeId>) -> Self {
        match (level, type_id) {
            (EntitlementLevelEnum::Type, Some(type_id)) => RowLevel::Type(type_id),
            (EntitlementLevelEnum::Customer, _) => RowLevel::Customer,
            _ => RowLevel::Feature,
        }
    }

    pub fn columns(self) -> (EntitlementLevelEnum, Option<ResourceTypeId>) {
        match self {
            RowLevel::Feature => (EntitlementLevelEnum::Feature, None),
            RowLevel::Customer => (EntitlementLevelEnum::Customer, None),
            RowLevel::Type(type_id) => (EntitlementLevelEnum::Type, Some(type_id)),
        }
    }
}

impl TryFrom<EntitlementRow> for Entitlement {
    type Error = StoreErrorReport;

    fn try_from(row: EntitlementRow) -> Result<Self, Self::Error> {
        let entity = EntitlementEntityId::from(&row);
        Ok(Entitlement {
            id: row.lineage_id,
            version_id: row.id,
            tenant_id: row.tenant_id,
            feature_id: row.feature_id,
            entity,
            effect: row.effect.into(),
            level: RowLevel::from_columns(row.level, row.level_type_id),
            resource_id: row.resource_id,
            value: from_json(row.value, "entitlement value")?,
            condition: row
                .condition
                .map(|c| from_json(c, "entitlement condition"))
                .transpose()?,
            valid_from: row.valid_from,
            valid_until: row.valid_until,
            reason: row.reason,
            policy_override: row
                .policy_override
                .map(|p| from_json(p, "policy override"))
                .transpose()?,
            created_by: row
                .created_by_kind
                .map(|kind| Actor::from_parts(ActorType::from(kind), row.created_by, None)),
            revoked_by: row.revoked_by,
            lineage_created_at: row.lineage_created_at,
            lineage_anchor: row.lineage_anchor,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        })
    }
}

impl Entitlement {
    pub fn tier(&self) -> Tier {
        Tier::of(&self.entity, self.resource_id.is_some())
    }

    /// The row as the fold reads it, its condition matched against the level's attributes.
    /// `units` multiply it when its tier counts per unit (R-133).
    pub fn fold_row(&self, quantity: u32, units: Option<u32>, attributes: &Attributes) -> FoldRow {
        let tier = self.tier();
        FoldRow {
            lineage_id: self.id,
            lineage_created_at: self.lineage_created_at,
            lineage_anchor: self.lineage_anchor,
            source: self.entity,
            tier,
            effect: self.effect,
            value: self.value.clone(),
            condition: self.condition.clone(),
            condition_met: self
                .condition
                .as_ref()
                .is_none_or(|c| c.matches(attributes)),
            quantity,
            units: units.filter(|_| tier.counts_per_unit()),
            policy_override: self.policy_override.clone(),
            created_at: self.created_at,
            valid_from: self.valid_from,
            valid_until: self.valid_until,
            revoked_at: self.revoked_at,
        }
    }
}

/// A row to create on an owner the caller supplies.
#[derive(Clone, Debug, PartialEq)]
pub struct EntitlementSpec {
    pub feature_id: FeatureId,
    pub value: GrantValue,
    /// `None` takes the tier's default (§3.2).
    pub effect: Option<Effect>,
    pub level: RowLevel,
    pub condition: Option<Condition>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    /// Set to rows of limits only.
    pub policy_override: Option<FeaturePolicy>,
}

impl EntitlementSpec {
    pub fn new(feature_id: FeatureId, value: GrantValue) -> Self {
        Self {
            feature_id,
            value,
            effect: None,
            level: RowLevel::Feature,
            condition: None,
            valid_from: None,
            valid_until: None,
            reason: None,
            policy_override: None,
        }
    }
}

/// A copy of a row onto another owner, e.g. a quote's rows onto its subscription.
impl From<&Entitlement> for EntitlementSpec {
    fn from(e: &Entitlement) -> Self {
        Self {
            feature_id: e.feature_id,
            value: e.value.clone(),
            effect: Some(e.effect),
            level: e.level,
            condition: e.condition.clone(),
            valid_from: e.valid_from,
            valid_until: e.valid_until,
            reason: e.reason.clone(),
            policy_override: e.policy_override.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct EntitlementNew {
    pub tenant_id: TenantId,
    pub entity: EntitlementEntityId,
    /// Customer rows only: a resource override on one of the customer's stored resources.
    pub resource_id: Option<ResourceId>,
    pub spec: EntitlementSpec,
}

/// An edit, applied to a successor of the open row. Owner, feature, level and resource stay.
#[derive(Clone, Debug, Default)]
pub struct EntitlementUpdate {
    pub value: Option<GrantValue>,
    pub effect: Option<Effect>,
    pub condition: Option<Option<Condition>>,
    pub valid_from: Option<Option<DateTime<Utc>>>,
    pub valid_until: Option<Option<DateTime<Utc>>>,
    pub reason: Option<Option<String>>,
    pub policy_override: Option<Option<FeaturePolicy>>,
}

/// The thresholds an API request asks for: `thresholds`, sorted and distinct, else the deprecated
/// single `warning_threshold_pct`. Writes validate the result.
pub fn requested_thresholds(
    mut thresholds: Vec<u32>,
    warning_threshold_pct: Option<u32>,
) -> Vec<u32> {
    if thresholds.is_empty() {
        return warning_threshold_pct.into_iter().collect();
    }
    thresholds.sort_unstable();
    thresholds.dedup();
    thresholds
}

/// The customer-wide readout of a feature (one value, policy terms spelled out), for the readers
/// that predate levels. Limits counted by the app have no metric and are not part of it.
#[derive(Clone, Debug)]
pub enum ResolvedEntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        metric_id: BillableMetricId,
        limit: Option<Decimal>,
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        thresholds: Vec<u32>,
        enabled: bool,
        group_by: Option<String>,
    },
    Config {
        value: ConfigValue,
    },
}

impl ResolvedEntitlementValue {
    /// `None` for a limit counted by the app.
    pub fn of(
        kind: &FeatureKind,
        value: &GrantValue,
        policy: &FeaturePolicy,
        subscribed: bool,
    ) -> Option<Self> {
        Some(match (kind, value) {
            (FeatureKind::Limit(LimitCountedBy::Events(metric_id)), GrantValue::Limit(limit)) => {
                Self::Metered {
                    metric_id: *metric_id,
                    limit: match limit {
                        LimitValue::Amount(n) => Some(*n),
                        LimitValue::Off | LimitValue::Unlimited => None,
                    },
                    reset_period: policy.reset_period(subscribed),
                    overage_behavior: policy.overage_behavior(),
                    thresholds: policy.thresholds.clone(),
                    enabled: *limit != LimitValue::Off,
                    group_by: None,
                }
            }
            (FeatureKind::Limit(_), _) => return None,
            (_, GrantValue::Boolean(enabled)) => Self::Boolean { enabled: *enabled },
            (_, GrantValue::Number(n)) => Self::Config {
                value: ConfigValue::Number(*n),
            },
            (_, GrantValue::Text(t)) => Self::Config {
                value: ConfigValue::Text(t.clone()),
            },
            (_, GrantValue::Json(j)) => Self::Config {
                value: ConfigValue::Json(j.clone()),
            },
            (_, GrantValue::Limit(_)) => return None,
        })
    }
}

/// The legacy readout with the usage of a limit.
#[derive(Clone, Debug)]
pub enum EffectiveEntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        metric_id: BillableMetricId,
        limit: Option<Decimal>,
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        thresholds: Vec<u32>,
        /// False once a blocking limit (plus its grace) is reached, even when switched on.
        enabled: bool,
        group_by: Option<String>,
        usage: EntitlementUsage,
        /// Switched on, but refused because a blocking limit (plus its grace) is reached.
        limit_reached: bool,
    },
    Config {
        value: ConfigValue,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureProductRef {
    pub id: ProductId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureRef {
    pub id: FeatureId,
    pub name: String,
    pub code: String,
    pub product: Option<FeatureProductRef>,
}

/// The source of the row the value comes from, with its display name when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedOrigin {
    pub entity: EntitlementEntityId,
    pub name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedEntitlement {
    pub feature: FeatureRef,
    pub value: ResolvedEntitlementValue,
    pub origin: ResolvedOrigin,
}

#[derive(Clone, Debug)]
pub struct EffectiveEntitlement {
    pub feature: FeatureRef,
    pub value: EffectiveEntitlementValue,
    pub origin: ResolvedOrigin,
    /// On a subscription's readout, when another subscription decides the value.
    pub own_grant: Option<SubscriptionGrant>,
}

/// What a subscription's own rows grant on a level another subscription decides (ADR 0011
/// §3.7): "grants 1k; Pro on the customer decides 10k".
#[derive(Clone, Debug, PartialEq)]
pub struct SubscriptionGrant {
    /// None when it includes the feature through its product alone.
    pub value: Option<GrantValue>,
    pub deciding_subscription_id: SubscriptionId,
    pub deciding_plan_name: Option<String>,
    /// The deciding subscription's resource (`site=s1`); none for the customer's own.
    pub deciding_resource_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn kinds_order_their_values() {
        let select = |ordered| {
            FeatureKind::Value(ValueType::Select {
                options: vec!["bronze".into(), "gold".into()],
                ordered,
            })
        };
        assert_eq!(
            select(true).value_order(),
            ValueOrder::Ranked(vec!["bronze".into(), "gold".into()])
        );
        assert!(select(false).value_order().is_unordered());
        assert!(
            FeatureKind::Value(ValueType::Json)
                .value_order()
                .is_unordered()
        );
        assert!(
            select(false)
                .check_value(&GrantValue::Text("silver".into()))
                .is_err()
        );
        assert!(
            FeatureKind::Limit(LimitCountedBy::App)
                .check_value(&GrantValue::Limit(LimitValue::Amount(dec!(-1))))
                .is_err()
        );
        assert!(
            FeatureKind::Boolean
                .check_value(&GrantValue::Number(dec!(1)))
                .is_err()
        );
    }

    #[test]
    fn app_counted_limits_never_reset() {
        let policy = FeatureKind::Limit(LimitCountedBy::App)
            .default_policy()
            .unwrap();
        assert_eq!(policy.validate(true), Ok(()));
    }
}
