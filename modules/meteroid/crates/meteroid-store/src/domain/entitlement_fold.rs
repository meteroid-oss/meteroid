//! The entitlement fold (ADR 0011 §3.2): one feature, one level, one deciding subscription, one value.
//!
//! Pure: callers load the rows of the level, match their conditions, and fold.
use crate::domain::entitlement_policy::FeaturePolicy;
use crate::domain::entitlements::ConfigValue;
use chrono::{DateTime, Utc};
use common_domain::ids::{EntitlementEntityId, EntitlementId, ResourceTypeId};
use rust_decimal::Decimal;
use std::cmp::{Ordering, Reverse};
use std::collections::BTreeMap;

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum Effect {
    Set,
    Add,
    RaiseTo,
    CapAt,
}

/// Where a row comes from, lowest first. Quotes share the subscription tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, strum::IntoStaticStr)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum Tier {
    Default,
    Plan,
    PlanVersion,
    Subscription,
    AddOn,
    Customer,
    ResourceOverride,
}

impl Tier {
    /// A customer row on one stored resource is a resource override.
    pub fn of(entity: &EntitlementEntityId, on_resource: bool) -> Self {
        match entity {
            EntitlementEntityId::Feature(_) => Tier::Default,
            EntitlementEntityId::Plan(_) => Tier::Plan,
            EntitlementEntityId::PlanVersion(_) => Tier::PlanVersion,
            EntitlementEntityId::Subscription(_) | EntitlementEntityId::Quote(_) => {
                Tier::Subscription
            }
            EntitlementEntityId::AddOn(_) => Tier::AddOn,
            EntitlementEntityId::Customer(_) if on_resource => Tier::ResourceOverride,
            EntitlementEntityId::Customer(_) => Tier::Customer,
        }
    }

    pub fn allowed_effects(self, order: &ValueOrder) -> &'static [Effect] {
        if order.is_unordered() {
            return &[Effect::Set];
        }
        match self {
            Tier::Default | Tier::Plan | Tier::PlanVersion | Tier::Subscription => &[Effect::Set],
            Tier::AddOn => &[Effect::Add, Effect::RaiseTo],
            Tier::Customer | Tier::ResourceOverride => {
                &[Effect::RaiseTo, Effect::Set, Effect::CapAt]
            }
        }
    }

    pub fn default_effect(self, order: &ValueOrder, multi_instance: bool) -> Effect {
        match self {
            _ if order.is_unordered() => Effect::Set,
            Tier::AddOn if multi_instance => Effect::Add,
            Tier::AddOn | Tier::Customer | Tier::ResourceOverride => Effect::RaiseTo,
            Tier::Default | Tier::Plan | Tier::PlanVersion | Tier::Subscription => Effect::Set,
        }
    }

    /// A limit counted per unit of a managed quantity (R-133) scales the subscription's grant:
    /// its default, plan and subscription rows. Add-ons keep their own quantity, overrides are
    /// absolute.
    pub fn counts_per_unit(self) -> bool {
        matches!(
            self,
            Tier::Default | Tier::Plan | Tier::PlanVersion | Tier::Subscription
        )
    }
}

/// A row's stored level (§3.3). `Feature` follows the feature's own level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowLevel {
    Feature,
    Customer,
    Type(ResourceTypeId),
}

/// A level a feature is evaluated at: the customer, or one resource type below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub enum EvalLevel {
    Customer,
    Type(ResourceTypeId),
}

impl RowLevel {
    /// The one spelling writes store: the feature's own level is `Feature`.
    pub fn normalised(self, feature_level: Option<ResourceTypeId>) -> Self {
        if self.eval(feature_level) == EvalLevel::of_feature(feature_level) {
            RowLevel::Feature
        } else {
            self
        }
    }

    pub fn eval(self, feature_level: Option<ResourceTypeId>) -> EvalLevel {
        match self {
            RowLevel::Feature => EvalLevel::of_feature(feature_level),
            RowLevel::Customer => EvalLevel::Customer,
            RowLevel::Type(id) => EvalLevel::Type(id),
        }
    }
}

impl EvalLevel {
    /// `feature.level_resource_type_id`, NULL being the customer.
    pub fn of_feature(feature_level: Option<ResourceTypeId>) -> Self {
        feature_level.map_or(EvalLevel::Customer, EvalLevel::Type)
    }

    pub fn type_id(self) -> Option<ResourceTypeId> {
        match self {
            EvalLevel::Customer => None,
            EvalLevel::Type(id) => Some(id),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LimitValue {
    Off,
    Amount(Decimal),
    Unlimited,
}

/// A row's value: only the number, the policy lives on the feature. Select values are `Text`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(into = "StoredValue", from = "StoredValue")]
pub enum GrantValue {
    Boolean(bool),
    Limit(LimitValue),
    Number(Decimal),
    Text(String),
    Json(serde_json::Value),
}

/// The stored shape, which keeps `main`'s tags so backfilled values read as they are.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<Decimal>,
        #[serde(default = "enabled")]
        enabled: bool,
    },
    Config {
        value: ConfigValue,
    },
}

fn enabled() -> bool {
    true
}

impl From<GrantValue> for StoredValue {
    fn from(value: GrantValue) -> Self {
        let config = |value| StoredValue::Config { value };
        match value {
            GrantValue::Boolean(enabled) => StoredValue::Boolean { enabled },
            GrantValue::Limit(limit) => StoredValue::Metered {
                limit: match limit {
                    LimitValue::Amount(n) => Some(n),
                    LimitValue::Off | LimitValue::Unlimited => None,
                },
                enabled: limit != LimitValue::Off,
            },
            GrantValue::Number(n) => config(ConfigValue::Number(n)),
            GrantValue::Text(t) => config(ConfigValue::Text(t)),
            GrantValue::Json(j) => config(ConfigValue::Json(j)),
        }
    }
}

impl From<StoredValue> for GrantValue {
    fn from(stored: StoredValue) -> Self {
        match stored {
            StoredValue::Boolean { enabled } => GrantValue::Boolean(enabled),
            StoredValue::Metered { enabled: false, .. } => GrantValue::Limit(LimitValue::Off),
            StoredValue::Metered { limit: None, .. } => GrantValue::Limit(LimitValue::Unlimited),
            StoredValue::Metered { limit: Some(n), .. } => GrantValue::Limit(LimitValue::Amount(n)),
            StoredValue::Config { value } => match value {
                ConfigValue::Boolean(b) => GrantValue::Boolean(b),
                ConfigValue::Number(n) => GrantValue::Number(n),
                ConfigValue::Text(t) => GrantValue::Text(t),
                ConfigValue::Json(j) => GrantValue::Json(j),
            },
        }
    }
}

/// How a feature's values compare, which every effect but Set relies on.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueOrder {
    /// off < 0 < n < unlimited.
    Limit,
    /// off < on.
    Boolean,
    Number {
        higher_is_better: bool,
    },
    /// An ordered select's options, worst first.
    Ranked(Vec<String>),
    /// Text, unordered select and json.
    Unordered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Off,
    Finite(Decimal),
    Unlimited,
}

impl ValueOrder {
    pub fn is_unordered(&self) -> bool {
        matches!(self, ValueOrder::Unordered)
    }

    /// Add sums these; on the others it raises at most.
    fn is_additive(&self) -> bool {
        matches!(
            self,
            ValueOrder::Limit
                | ValueOrder::Number {
                    higher_is_better: true
                }
        )
    }

    pub fn is_off(&self, value: &GrantValue) -> bool {
        self.rank(value) == Some(Rank::Off)
    }

    /// `Greater` when `a` is better than `b`; `None` when they can't be compared.
    pub fn compare(&self, a: &GrantValue, b: &GrantValue) -> Option<Ordering> {
        Some(self.rank(a)?.cmp(&self.rank(b)?))
    }

    fn rank(&self, value: &GrantValue) -> Option<Rank> {
        match (self, value) {
            (ValueOrder::Boolean, GrantValue::Boolean(false)) => Some(Rank::Off),
            (ValueOrder::Boolean, GrantValue::Boolean(true)) => Some(Rank::Finite(Decimal::ONE)),
            (ValueOrder::Limit, GrantValue::Limit(LimitValue::Off)) => Some(Rank::Off),
            (ValueOrder::Limit, GrantValue::Limit(LimitValue::Amount(n))) => Some(Rank::Finite(*n)),
            (ValueOrder::Limit, GrantValue::Limit(LimitValue::Unlimited)) => Some(Rank::Unlimited),
            (ValueOrder::Number { higher_is_better }, GrantValue::Number(n)) => {
                Some(Rank::Finite(if *higher_is_better { *n } else { -*n }))
            }
            (ValueOrder::Ranked(options), GrantValue::Text(t)) => options
                .iter()
                .position(|o| o == t)
                .map(|i| Rank::Finite(Decimal::from(i as u64))),
            _ => None,
        }
    }
}

/// `{"<type code>.<attribute>": [values]}`: every attribute must hold one of its values.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Condition(pub BTreeMap<String, Vec<String>>);

/// More attributes first, then narrower lists.
pub type Specificity = (usize, Reverse<usize>);

impl Condition {
    /// An attribute the context lacks does not match.
    pub fn matches(&self, attributes: &BTreeMap<String, String>) -> bool {
        self.0.iter().all(|(attribute, allowed)| {
            attributes
                .get(attribute)
                .is_some_and(|v| allowed.contains(v))
        })
    }

    pub fn specificity(&self) -> Specificity {
        (self.0.len(), Reverse(self.0.values().map(Vec::len).sum()))
    }

    /// Hex sha256 of the canonical JSON (sorted keys and values), `""` for an empty condition.
    pub fn hash(&self) -> String {
        use sha2::{Digest, Sha256};
        if self.0.is_empty() {
            return String::new();
        }
        let mut canonical = self.0.clone();
        for values in canonical.values_mut() {
            values.sort();
            values.dedup();
        }
        let json = serde_json::to_vec(&canonical).expect("a map of strings serialises");
        hex::encode(Sha256::digest(json))
    }

    /// `entitlement.condition_hash`.
    pub fn hash_of(condition: Option<&Condition>) -> String {
        condition.map(Condition::hash).unwrap_or_default()
    }
}

/// One live-or-not row of the level being folded.
#[derive(Clone, Debug)]
pub struct FoldRow {
    pub lineage_id: EntitlementId,
    /// The age used for ties.
    pub lineage_created_at: DateTime<Utc>,
    /// See `Entitlement::lineage_anchor`.
    pub lineage_anchor: DateTime<Utc>,
    pub source: EntitlementEntityId,
    pub tier: Tier,
    pub effect: Effect,
    pub value: GrantValue,
    pub condition: Option<Condition>,
    /// `condition.matches(attributes of the level's resource)`; true without a condition.
    pub condition_met: bool,
    /// Add-on rows: the instances in effect, or a seat add-on's seats held. 1 elsewhere.
    pub quantity: u32,
    /// A limit counted per unit of a managed quantity: the units the deciding subscription
    /// holds, which multiply the row's amount (R-133).
    pub units: Option<u32>,
    pub policy_override: Option<FeaturePolicy>,
    pub created_at: DateTime<Utc>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl FoldRow {
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.created_at <= now
            && self.valid_from.is_none_or(|from| from <= now)
            && self.valid_until.is_none_or(|until| now < until)
            && self.revoked_at.is_none_or(|revoked| now < revoked)
    }

    fn specificity(&self) -> Specificity {
        self.condition
            .as_ref()
            .map_or((0, Reverse(0)), Condition::specificity)
    }

    /// The value the row grants: its amount times `units` for a limit counted per unit.
    pub fn granted(&self) -> GrantValue {
        match (&self.value, self.units) {
            (GrantValue::Limit(LimitValue::Amount(n)), Some(units)) => GrantValue::Limit(
                n.checked_mul(Decimal::from(units))
                    .map_or(LimitValue::Unlimited, LimitValue::Amount),
            ),
            (value, _) => value.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::IntoStaticStr)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum Discarded {
    ReplacedByHigherTier,
    /// Another Set to of its tier won: a more specific condition, a better value, or an older lineage.
    LostWithinTier,
    /// It changed nothing, a Cap at over nothing included.
    NoEffect,
    /// An Add or Raise to on a level that has nothing to add to.
    NoBaseOnLevel,
    EffectNotAllowed,
    ConditionNotMet,
    NotInEffect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Contribution {
    pub lineage_id: EntitlementId,
    pub source: EntitlementEntityId,
    /// Filled by the resolver for display: "Growth v3", "Extra projects".
    pub source_name: Option<String>,
    pub tier: Tier,
    pub effect: Effect,
    pub value: GrantValue,
    pub condition: Option<Condition>,
    pub quantity: u32,
    /// The units held that multiplied `value` (R-133); `running` is the product.
    pub units: Option<u32>,
    pub policy_override: bool,
    /// The value after this row; `None` when it was discarded.
    pub running: Option<GrantValue>,
    pub discarded: Option<Discarded>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folded {
    /// `None`: not entitled on the feature's own level, unconstrained on another.
    pub value: Option<GrantValue>,
    pub policy: FeaturePolicy,
    /// The Set to row whose override is in effect; `None` for the feature's policy.
    pub policy_from: Option<EntitlementId>,
    pub explain: Vec<Contribution>,
}

enum Acc {
    Nothing,
    Unconstrained,
    Value(GrantValue),
}

/// Folds the rows of one level (§3.2). `None` only without rows.
pub fn fold(
    order: &ValueOrder,
    own_level: bool,
    feature_policy: &FeaturePolicy,
    rows: &[FoldRow],
    now: DateTime<Utc>,
) -> Option<Folded> {
    if rows.is_empty() {
        return None;
    }
    let mut by_tier: BTreeMap<Tier, Vec<&FoldRow>> = BTreeMap::new();
    for row in rows {
        by_tier.entry(row.tier).or_default().push(row);
    }

    let mut acc = if own_level {
        Acc::Nothing
    } else {
        Acc::Unconstrained
    };
    let mut policy = feature_policy.clone();
    let mut policy_from = None;
    let mut explain = Vec::new();

    for (tier, mut tier_rows) in by_tier {
        tier_rows.sort_by_key(|r| (r.lineage_created_at, *r.lineage_id));
        let allowed = tier.allowed_effects(order);
        let mut kept = Vec::new();
        for row in tier_rows {
            let skipped = if !row.is_live(now) {
                Some(Discarded::NotInEffect)
            } else if !row.condition_met {
                Some(Discarded::ConditionNotMet)
            } else if !allowed.contains(&row.effect) {
                Some(Discarded::EffectNotAllowed)
            } else {
                None
            };
            match skipped {
                Some(cause) => explain.push(contribution(row, Err(cause))),
                None => kept.push(row),
            }
        }

        let sets: Vec<&FoldRow> = kept
            .iter()
            .copied()
            .filter(|r| r.effect == Effect::Set)
            .collect();
        if let Some(winner) = pick_set(order, &sets) {
            for c in explain.iter_mut().filter(|c| c.running.is_some()) {
                c.running = None;
                c.discarded = Some(Discarded::ReplacedByHigherTier);
            }
            acc = Acc::Value(winner.granted());
            if let Some(over) = &winner.policy_override {
                policy = over.clone();
                policy_from = Some(winner.lineage_id);
            }
            for row in &sets {
                let outcome = if row.lineage_id == winner.lineage_id {
                    Ok(winner.granted())
                } else {
                    Err(Discarded::LostWithinTier)
                };
                explain.push(contribution(row, outcome));
            }
        }

        for effect in [Effect::Add, Effect::RaiseTo, Effect::CapAt] {
            for row in kept.iter().filter(|r| r.effect == effect) {
                let outcome = apply(order, &acc, row).and_then(|next| match &acc {
                    Acc::Value(current) if *current == next => Err(Discarded::NoEffect),
                    _ => Ok(next),
                });
                if let Ok(next) = &outcome {
                    acc = Acc::Value(next.clone());
                }
                explain.push(contribution(row, outcome));
            }
        }
    }

    Some(Folded {
        value: match acc {
            Acc::Value(value) => Some(value),
            Acc::Nothing | Acc::Unconstrained => None,
        },
        policy,
        policy_from,
        explain,
    })
}

fn apply(order: &ValueOrder, acc: &Acc, row: &FoldRow) -> Result<GrantValue, Discarded> {
    let current = match acc {
        Acc::Value(v) => Some(v),
        _ => None,
    };
    let value = row.granted();
    match row.effect {
        Effect::CapAt => match acc {
            Acc::Nothing => Err(Discarded::NoEffect),
            Acc::Unconstrained => Ok(value),
            Acc::Value(c) if order.compare(&value, c) == Some(Ordering::Less) => Ok(value),
            Acc::Value(_) => Err(Discarded::NoEffect),
        },
        _ if order.is_off(&value) || row.quantity == 0 => Err(Discarded::NoEffect),
        _ if matches!(acc, Acc::Unconstrained) => Err(Discarded::NoBaseOnLevel),
        Effect::Add if order.is_additive() => {
            added(current, &value, row.quantity).ok_or(Discarded::NoEffect)
        }
        _ => match current {
            Some(c) if order.compare(&value, c) != Some(Ordering::Greater) => {
                Err(Discarded::NoEffect)
            }
            _ => Ok(value),
        },
    }
}

/// `acc + value × quantity`; off or nothing starts from 0, unlimited absorbs, overflow is unlimited.
fn added(acc: Option<&GrantValue>, value: &GrantValue, quantity: u32) -> Option<GrantValue> {
    let times =
        |n: &Decimal, base: Decimal| n.checked_mul(Decimal::from(quantity))?.checked_add(base);
    match (acc, value) {
        (Some(GrantValue::Limit(LimitValue::Unlimited)), _)
        | (_, GrantValue::Limit(LimitValue::Unlimited)) => {
            Some(GrantValue::Limit(LimitValue::Unlimited))
        }
        (acc, GrantValue::Limit(LimitValue::Amount(n))) => {
            let base = match acc {
                Some(GrantValue::Limit(LimitValue::Amount(b))) => *b,
                _ => Decimal::ZERO,
            };
            Some(GrantValue::Limit(
                times(n, base).map_or(LimitValue::Unlimited, LimitValue::Amount),
            ))
        }
        (acc, GrantValue::Number(n)) => {
            let base = match acc {
                Some(GrantValue::Number(b)) => *b,
                _ => Decimal::ZERO,
            };
            Some(GrantValue::Number(times(n, base).unwrap_or(Decimal::MAX)))
        }
        _ => None,
    }
}

/// The most specific condition, then the best value, then the oldest lineage (rows come oldest first).
fn pick_set<'a>(order: &ValueOrder, sets: &[&'a FoldRow]) -> Option<&'a FoldRow> {
    let top = sets.iter().map(|r| r.specificity()).max()?;
    sets.iter()
        .copied()
        .filter(|r| r.specificity() == top)
        .reduce(
            |best, r| match order.compare(&r.granted(), &best.granted()) {
                Some(Ordering::Greater) => r,
                _ => best,
            },
        )
}

fn contribution(row: &FoldRow, outcome: Result<GrantValue, Discarded>) -> Contribution {
    Contribution {
        lineage_id: row.lineage_id,
        source: row.source,
        source_name: None,
        tier: row.tier,
        effect: row.effect,
        value: row.value.clone(),
        condition: row.condition.clone(),
        quantity: row.quantity,
        units: row.units,
        policy_override: row.policy_override.is_some(),
        running: outcome.as_ref().ok().cloned(),
        discarded: outcome.err(),
    }
}

#[cfg(test)]
#[path = "entitlement_fold_tests.rs"]
mod tests;
