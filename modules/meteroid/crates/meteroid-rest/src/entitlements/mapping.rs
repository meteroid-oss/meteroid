use common_domain::actor::Actor;
use common_domain::ids::{BaseId, EntitlementEntityId, ResourceTypeId};
use meteroid_store::domain::entitlement_checks::{
    AppCountExplain as DomainAppCountExplain, CheckLevel, CheckLevelExplain, CheckResponse,
    DecidingCandidate as DomainCandidate, LevelValue, Snapshot, SnapshotFeature,
    SnapshotLevel as DomainSnapshotLevel, UsageSource as DomainUsageSource,
};
use meteroid_store::domain::entitlement_fold::{
    Condition, Contribution, EvalLevel, GrantValue, LimitValue, RowLevel,
};
use meteroid_store::domain::entitlement_levels::CheckReason;
use meteroid_store::domain::entitlement_policy::{
    FeaturePolicy as DomainPolicy, LegacyLimitTerms, Resets,
};
use meteroid_store::domain::entitlements::{self as domain};
use meteroid_store::services::entitlements::app_counts::AppCountResult;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

use super::model::*;
use crate::webhooks::out_model::WebhookOutActorType;

fn feature_ref(f: domain::FeatureRef) -> FeatureRef {
    FeatureRef {
        id: f.id,
        name: f.name,
        code: f.code,
        product: f.product.map(|p| ProductRef {
            id: p.id,
            name: p.name,
        }),
    }
}

fn kind_to_rest(
    kind: domain::FeatureKind,
) -> (
    FeatureKind,
    Option<FeatureValueSpec>,
    Option<FeatureLimitSpec>,
) {
    use domain::ValueType as V;
    let value = |value_type, higher_is_better, options, ordered| FeatureValueSpec {
        value_type,
        higher_is_better,
        options,
        ordered,
    };
    match kind {
        domain::FeatureKind::Boolean => (FeatureKind::Boolean, None, None),
        domain::FeatureKind::Value(v) => {
            let spec = match v {
                V::Number { higher_is_better } => value(
                    FeatureValueType::Number,
                    Some(higher_is_better),
                    vec![],
                    false,
                ),
                V::Text => value(FeatureValueType::Text, None, vec![], false),
                V::Select { options, ordered } => {
                    value(FeatureValueType::Select, None, options, ordered)
                }
                V::Json => value(FeatureValueType::Json, None, vec![], false),
            };
            (FeatureKind::Value, Some(spec), None)
        }
        domain::FeatureKind::Limit(counted_by) => {
            let (counted_by, metric_id) = match counted_by {
                domain::LimitCountedBy::Events(metric_id) => {
                    (LimitCountedBy::Events, Some(metric_id))
                }
                domain::LimitCountedBy::App => (LimitCountedBy::App, None),
            };
            let spec = FeatureLimitSpec {
                counted_by,
                metric_id,
            };
            (FeatureKind::Limit, None, Some(spec))
        }
    }
}

/// The request was validated: a kind comes with its own spec only.
pub fn kind_to_domain(
    kind: FeatureKind,
    value: Option<FeatureValueSpec>,
    limit: Option<FeatureLimitSpec>,
) -> Result<domain::FeatureKind, String> {
    use domain::ValueType as V;
    Ok(match (kind, value, limit) {
        (FeatureKind::Value, Some(v), _) => domain::FeatureKind::Value(match v.value_type {
            FeatureValueType::Number => V::Number {
                higher_is_better: v.higher_is_better.unwrap_or(true),
            },
            FeatureValueType::Text => V::Text,
            FeatureValueType::Select => V::Select {
                options: v.options,
                ordered: v.ordered,
            },
            FeatureValueType::Json => V::Json,
        }),
        (FeatureKind::Limit, _, Some(l)) => {
            domain::FeatureKind::Limit(match (l.counted_by, l.metric_id) {
                (LimitCountedBy::Events, Some(metric_id)) => {
                    domain::LimitCountedBy::Events(metric_id)
                }
                (LimitCountedBy::App, None) => domain::LimitCountedBy::App,
                _ => {
                    return Err(
                        "a limit counted by EVENTS takes a metric_id, one counted by APP none"
                            .into(),
                    );
                }
            })
        }
        (FeatureKind::Boolean, None, None) => domain::FeatureKind::Boolean,
        _ => return Err("VALUE takes `value`, LIMIT takes `limit`, BOOLEAN neither".into()),
    })
}

/// The kind a v1 `feature_type` maps to, as the entitlements_v2 migration converts features.
fn feature_type_to_kind(feature_type: FeatureType) -> domain::FeatureKind {
    use domain::ValueType as V;
    match feature_type {
        FeatureType::Boolean(_) => domain::FeatureKind::Boolean,
        FeatureType::Metered(m) => {
            domain::FeatureKind::Limit(domain::LimitCountedBy::Events(m.metric_id))
        }
        FeatureType::Config(c) => match c.value_type {
            ConfigValueType::Boolean => domain::FeatureKind::Boolean,
            ConfigValueType::Number => domain::FeatureKind::Value(V::Number {
                higher_is_better: true,
            }),
            ConfigValueType::Select => domain::FeatureKind::Value(V::Select {
                options: c.options,
                ordered: false,
            }),
            ConfigValueType::Map | ConfigValueType::Json => domain::FeatureKind::Value(V::Json),
            ConfigValueType::Text => domain::FeatureKind::Value(V::Text),
        },
    }
}

/// A new feature's kind: `kind` with its spec, or the deprecated `feature_type`; both must agree.
pub fn created_feature_kind(
    kind: Option<FeatureKind>,
    value: Option<FeatureValueSpec>,
    limit: Option<FeatureLimitSpec>,
    feature_type: Option<FeatureType>,
) -> Result<domain::FeatureKind, String> {
    match (kind, feature_type) {
        (Some(kind), None) => kind_to_domain(kind, value, limit),
        (None, Some(_)) if value.is_some() || limit.is_some() => {
            Err("`value` and `limit` go with `kind`, not with the deprecated `feature_type`".into())
        }
        (None, Some(feature_type)) => Ok(feature_type_to_kind(feature_type)),
        (Some(kind), Some(feature_type)) => {
            let kind = kind_to_domain(kind, value, limit)?;
            if kind == feature_type_to_kind(feature_type) {
                Ok(kind)
            } else {
                Err(
                    "`kind` and the deprecated `feature_type` describe different features: \
                     send `kind` alone"
                        .into(),
                )
            }
        }
        (None, None) => Err("`kind` is required".into()),
    }
}

/// The deprecated limit terms a v1 value carries, if any; `group_by` is refused.
pub fn legacy_terms(value: &EntitlementValue) -> Result<Option<LegacyLimitTerms>, String> {
    let EntitlementValue::Metered(m) = value else {
        return Ok(None);
    };
    if matches!(m.group_by, Some(Some(_))) {
        return Err(
            "`group_by` is no longer supported: a limit applies per resource instead \
                    (the feature's `level`)"
                .into(),
        );
    }
    let thresholds = match (&m.thresholds, m.warning_threshold_pct) {
        (None, None) => None,
        (thresholds, warning) => Some(domain::requested_thresholds(
            thresholds.clone().flatten().unwrap_or_default(),
            warning.flatten(),
        )),
    };
    let terms = LegacyLimitTerms {
        reset_period: m.reset_period.clone().map(Into::into),
        allow_overage: m
            .overage_behavior
            .map(|o| o == Some(OverageBehavior::Allow)),
        grace_pct: m.grace_pct.map(Option::unwrap_or_default),
        thresholds,
    };
    Ok((terms != LegacyLimitTerms::default()).then_some(terms))
}

/// What a row's deprecated terms make of its `policy_override`: `None` leaves it, `Some` writes
/// it. `in_effect` is the row's own override, else the feature's policy. A Set to row takes the
/// terms as its override, as the migration converts rows; another limited row can't hold them.
pub fn override_for_terms(
    feature: &domain::Feature,
    in_effect: &DomainPolicy,
    terms: &LegacyLimitTerms,
    is_set: bool,
    value: &GrantValue,
) -> Result<Option<Option<DomainPolicy>>, String> {
    let requested = in_effect.with_legacy_terms(terms);
    // A value of another kind is refused by the kind check.
    if requested == *in_effect || !matches!(feature.kind, domain::FeatureKind::Limit(_)) {
        return Ok(None);
    }
    if is_set {
        let feature_policy = feature.policy_or_default();
        return Ok(Some((requested != feature_policy).then_some(requested)));
    }
    if !matches!(value, GrantValue::Limit(LimitValue::Amount(_))) {
        // Without an amount the terms act on nothing, as the migration treats such rows.
        return Ok(None);
    }
    Err(format!(
        "feature {}: `reset_period`, `overage_behavior`, `grace_pct` and `thresholds` are its \
         `policy`, which only a SET row overrides (`policy_override`); this row follows it, so \
         send terms equal to it or none",
        feature.code
    ))
}

/// A new feature's policy: `policy`, else the one the deprecated terms of its default ask for.
/// The `feature_type` shape reads omitted terms as v1 did: never reset, refused at the limit.
pub fn created_feature_policy(
    kind: &domain::FeatureKind,
    policy: Option<DomainPolicy>,
    terms: Option<LegacyLimitTerms>,
    v1_defaults: bool,
) -> Result<Option<DomainPolicy>, String> {
    if (terms.is_none() && !v1_defaults) || !matches!(kind, domain::FeatureKind::Limit(_)) {
        return Ok(policy);
    }
    let base = match (&policy, v1_defaults) {
        (Some(policy), _) => policy.clone(),
        (None, true) => DomainPolicy {
            resets: Resets::Never,
            ..DomainPolicy::default()
        },
        (None, false) => kind.default_policy().unwrap_or_default(),
    };
    let requested = base.with_legacy_terms(&terms.unwrap_or_default());
    match policy {
        Some(policy) if policy != requested => Err(
            "`policy` and the deprecated limit terms of `entitlement` disagree: send `policy` alone"
                .into(),
        ),
        _ => Ok(Some(requested)),
    }
}

/// The read-only legacy type; `None` for a limit counted by the app, which has no metric.
fn feature_type_of(kind: &domain::FeatureKind) -> Option<FeatureType> {
    use domain::ValueType as V;
    let config = |value_type, options: &[String]| {
        FeatureType::Config(ConfigFeatureType {
            value_type,
            options: options.to_vec(),
        })
    };
    Some(match kind {
        domain::FeatureKind::Boolean => FeatureType::Boolean(BooleanFeatureType {}),
        domain::FeatureKind::Limit(domain::LimitCountedBy::Events(metric_id)) => {
            FeatureType::Metered(MeteredFeatureType {
                metric_id: *metric_id,
            })
        }
        domain::FeatureKind::Limit(domain::LimitCountedBy::App) => return None,
        domain::FeatureKind::Value(V::Number { .. }) => config(ConfigValueType::Number, &[]),
        domain::FeatureKind::Value(V::Text) => config(ConfigValueType::Text, &[]),
        domain::FeatureKind::Value(V::Select { options, .. }) => {
            config(ConfigValueType::Select, options)
        }
        domain::FeatureKind::Value(V::Json) => config(ConfigValueType::Json, &[]),
    })
}

pub fn feature_to_rest(f: domain::Feature) -> Feature {
    let feature_type = feature_type_of(&f.kind);
    let (kind, value, limit) = kind_to_rest(f.kind);
    let defaults: Vec<_> = f.defaults.into_iter().map(entitlement_to_rest).collect();
    Feature {
        id: f.id,
        name: f.name,
        code: f.code,
        description: f.description,
        kind,
        value,
        limit,
        unit: f.unit,
        level: f.level_type_id,
        policy: f.policy.map(Into::into),
        feature_type,
        status: f.status.into(),
        product: f.product.map(|p| ProductRef {
            id: p.id,
            name: p.name,
        }),
        created_at: f.created_at.into(),
        entitlement: defaults.first().cloned(),
        defaults,
    }
}

pub fn grant_value_to_rest(v: GrantValue) -> EntitlementValue {
    let config = |value: domain::ConfigValue| {
        EntitlementValue::Config(ConfigEntitlementValue {
            value: value.into(),
        })
    };
    match v {
        GrantValue::Boolean(enabled) => {
            EntitlementValue::Boolean(BooleanEntitlementValue { enabled })
        }
        GrantValue::Limit(limit) => EntitlementValue::Metered(MeteredEntitlementValue {
            limit: match limit {
                LimitValue::Amount(n) => Some(n),
                LimitValue::Off | LimitValue::Unlimited => None,
            },
            enabled: limit != LimitValue::Off,
            reset_period: None,
            overage_behavior: None,
            grace_pct: None,
            warning_threshold_pct: None,
            thresholds: None,
            group_by: None,
        }),
        GrantValue::Number(n) => config(domain::ConfigValue::Number(n)),
        GrantValue::Text(t) => config(domain::ConfigValue::Text(t)),
        GrantValue::Json(j) => config(domain::ConfigValue::Json(j)),
    }
}

pub fn value_to_domain(v: EntitlementValue) -> GrantValue {
    match v {
        EntitlementValue::Boolean(b) => GrantValue::Boolean(b.enabled),
        EntitlementValue::Metered(m) if !m.enabled => GrantValue::Limit(LimitValue::Off),
        EntitlementValue::Metered(m) => {
            GrantValue::Limit(m.limit.map_or(LimitValue::Unlimited, LimitValue::Amount))
        }
        EntitlementValue::Config(c) => match config_value_to_domain(c.value) {
            domain::ConfigValue::Boolean(b) => GrantValue::Boolean(b),
            domain::ConfigValue::Number(n) => GrantValue::Number(n),
            domain::ConfigValue::Text(t) => GrantValue::Text(t),
            domain::ConfigValue::Json(j) => GrantValue::Json(j),
        },
    }
}

/// The value of a `VALUE` feature, as the check returns it.
fn config_of(v: GrantValue) -> Option<ConfigValue> {
    match v {
        GrantValue::Number(n) => Some(domain::ConfigValue::Number(n).into()),
        GrantValue::Text(t) => Some(domain::ConfigValue::Text(t).into()),
        GrantValue::Json(j) => Some(domain::ConfigValue::Json(j).into()),
        GrantValue::Boolean(_) | GrantValue::Limit(_) => None,
    }
}

fn row_level_to_rest(level: RowLevel) -> (EntitlementLevel, Option<ResourceTypeId>) {
    match level {
        RowLevel::Feature => (EntitlementLevel::Feature, None),
        RowLevel::Customer => (EntitlementLevel::Customer, None),
        RowLevel::Type(id) => (EntitlementLevel::Type, Some(id)),
    }
}

fn row_level(level: EntitlementLevel, type_id: Option<ResourceTypeId>) -> Result<RowLevel, String> {
    match (level, type_id) {
        (EntitlementLevel::Feature, None) => Ok(RowLevel::Feature),
        (EntitlementLevel::Customer, None) => Ok(RowLevel::Customer),
        (EntitlementLevel::Type, Some(id)) => Ok(RowLevel::Type(id)),
        _ => Err("level_type_id is required by the TYPE level, and refused by the others".into()),
    }
}

/// Actors as the `Actor` schema of the webhooks.
pub fn actor_to_rest(actor: Actor) -> EntitlementActor {
    let (actor_type, id) = match actor {
        Actor::User { id } => (WebhookOutActorType::User, Some(id.to_string())),
        Actor::ApiToken { id } => (WebhookOutActorType::ApiKey, Some(id.to_string())),
        Actor::OAuthApp { id } => (WebhookOutActorType::OauthApp, Some(id.to_string())),
        Actor::Workflow { id } => (WebhookOutActorType::System, Some(id.as_base62())),
        Actor::Customer { id } => (WebhookOutActorType::Customer, Some(id.to_string())),
        Actor::System | Actor::QuoteRecipient { .. } => (WebhookOutActorType::System, None),
    };
    EntitlementActor { actor_type, id }
}

pub fn entitlement_to_rest(e: domain::Entitlement) -> Entitlement {
    let (level, level_type_id) = row_level_to_rest(e.level);
    Entitlement {
        id: e.id,
        feature_id: e.feature_id,
        value: grant_value_to_rest(e.value),
        effect: e.effect.into(),
        level,
        level_type_id,
        resource_id: e.resource_id,
        condition: e.condition.map(|c| EntitlementCondition(c.0)),
        valid_from: e.valid_from.map(Into::into),
        valid_until: e.valid_until.map(Into::into),
        reason: e.reason,
        policy_override: e.policy_override.map(Into::into),
        created_by: e.created_by.map(actor_to_rest),
        created_at: e.lineage_created_at.into(),
        updated_at: e.created_at.into(),
        revoked_at: e.revoked_at.map(Into::into),
        revoked_by: e.revoked_by.map(|id| id.to_string()),
    }
}

pub fn entitlement_spec_to_domain(
    spec: EntitlementSpecRequest,
) -> Result<domain::EntitlementSpec, String> {
    Ok(domain::EntitlementSpec {
        effect: spec.effect.map(Into::into),
        level: row_level(spec.level, spec.level_type_id)?,
        condition: spec.condition.map(|c| Condition(c.0)),
        valid_from: spec.valid_from.map(Into::into),
        valid_until: spec.valid_until.map(Into::into),
        reason: spec.reason,
        policy_override: spec.policy_override.map(Into::into),
        ..domain::EntitlementSpec::new(spec.feature_id, value_to_domain(spec.value))
    })
}

pub fn entitlement_update_to_domain(r: UpdateEntitlementRequest) -> domain::EntitlementUpdate {
    domain::EntitlementUpdate {
        value: r.value.map(value_to_domain),
        effect: r.effect.map(Into::into),
        condition: r.condition.map(|c| c.map(|c| Condition(c.0))),
        valid_from: r.valid_from.map(|t| t.map(Into::into)),
        valid_until: r.valid_until.map(|t| t.map(Into::into)),
        reason: r.reason,
        policy_override: r.policy_override.map(|p| p.map(Into::into)),
    }
}

/// v1's released names: a level switched off is `DISABLED`, one granting nothing `NO_ENTITLEMENT`.
fn check_reason(reason: CheckReason, value: Option<&GrantValue>) -> EntitlementCheckReason {
    match reason {
        CheckReason::NotEntitled => match value {
            Some(GrantValue::Boolean(false) | GrantValue::Limit(LimitValue::Off)) => {
                EntitlementCheckReason::Disabled
            }
            _ => EntitlementCheckReason::NoEntitlement,
        },
        CheckReason::CreditExhausted => EntitlementCheckReason::InsufficientBalance,
        CheckReason::LimitReached => EntitlementCheckReason::LimitReached,
        CheckReason::BudgetExceeded => EntitlementCheckReason::BudgetExceeded,
        CheckReason::ResourceRequired => EntitlementCheckReason::ResourceRequired,
        CheckReason::NotSubscribed => EntitlementCheckReason::NotSubscribed,
        CheckReason::MetricNotFound => EntitlementCheckReason::MetricNotFound,
        CheckReason::UsageUnavailable => EntitlementCheckReason::UsageUnavailable,
    }
}

fn entity_ref(entity: EntitlementEntityId) -> String {
    match entity {
        EntitlementEntityId::Feature(id) => id.to_string(),
        EntitlementEntityId::Plan(id) => id.to_string(),
        EntitlementEntityId::PlanVersion(id) => id.to_string(),
        EntitlementEntityId::AddOn(id) => id.to_string(),
        EntitlementEntityId::Subscription(id) => id.to_string(),
        EntitlementEntityId::Quote(id) => id.to_string(),
        EntitlementEntityId::Customer(id) => id.to_string(),
    }
}

fn contribution_to_rest(c: Contribution) -> EntitlementContribution {
    EntitlementContribution {
        entitlement_id: c.lineage_id,
        source: entity_ref(c.source),
        source_name: c.source_name,
        tier: c.tier.into(),
        effect: c.effect.into(),
        value: grant_value_to_rest(c.value),
        condition: c.condition.map(|c| EntitlementCondition(c.0)),
        quantity: c.quantity,
        units: c.units,
        policy_override: c.policy_override,
        running: c.running.map(grant_value_to_rest),
        discarded: c.discarded.map(Into::into),
    }
}

fn app_count_explain_to_rest(a: DomainAppCountExplain) -> AppCountExplain {
    AppCountExplain {
        path: a.path,
        count: a.count,
        last_set_value: a.last_set_value,
        last_set_at: a.last_set_at.map(Into::into),
        last_set_key: a.last_set_key,
        adds_since: a.adds_since,
        adds_sum: a.adds_sum,
        last_add_at: a.last_add_at.map(Into::into),
    }
}

fn candidate_to_rest(c: DomainCandidate) -> DecidingCandidate {
    DecidingCandidate {
        subscription_id: c.subscription_id,
        resource_path: c.resource_path,
        value: c.value.map(grant_value_to_rest),
    }
}

fn level_explain_to_rest(e: CheckLevelExplain) -> LevelExplain {
    LevelExplain {
        contributions: e
            .contributions
            .into_iter()
            .map(contribution_to_rest)
            .collect(),
        candidates: e.candidates.into_iter().map(candidate_to_rest).collect(),
        app_counts: e
            .app_counts
            .into_iter()
            .map(app_count_explain_to_rest)
            .collect(),
    }
}

/// Each resource type's code, by id.
pub type TypeCodes = std::collections::HashMap<ResourceTypeId, String>;

fn level_code(level: EvalLevel, codes: &TypeCodes) -> Option<String> {
    level.type_id().and_then(|t| codes.get(&t).cloned())
}

pub fn level_to_rest(l: CheckLevel, codes: &TypeCodes) -> EntitlementLevelState {
    let reason = l
        .denial
        .map(|reason| check_reason(reason, l.value.as_ref()));
    EntitlementLevelState {
        level: level_code(l.level, codes),
        path: l.path,
        value: l.value.map(grant_value_to_rest),
        used: l.used,
        limit: l.limit,
        near_limit: l.near_limit,
        in_overage: l.in_overage,
        reset_at: l.reset_at.map(Into::into),
        deciding_subscription_id: l.deciding_subscription,
        allowed: l.denial.is_none(),
        reason,
        explain: l.explain.map(level_explain_to_rest),
    }
}

/// `used`, `limit` and `remaining` keep the shape of the earlier check: the denying level's, else
/// the feature's own (first).
pub fn check_to_rest(c: CheckResponse, codes: &TypeCodes) -> EntitlementCheck {
    let primary = c
        .levels
        .iter()
        .find(|l| l.denial.is_some())
        .or(c.levels.first());
    let used = primary.and_then(|l| l.used);
    let limit = primary.and_then(|l| l.limit);
    let period = primary
        .filter(|l| l.limit.is_some())
        .and_then(|l| l.window.as_ref())
        .map(|w| EntitlementCheckPeriod {
            start: w.start.into(),
            end: w.end.map(Into::into),
        });
    let reason = c
        .reason
        .map_or(EntitlementCheckReason::WithinLimit, |reason| {
            let denying = c.levels.iter().find(|l| l.denial == Some(reason));
            check_reason(reason, denying.and_then(|l| l.value.as_ref()))
        });
    let as_of = common_clock::now();
    let retry_after_ms = match c.retry_after {
        Some(at) => Some((at - as_of).num_milliseconds().max(0)),
        None if c.allowed && c.near_limit => Some(1000),
        None => None,
    };
    EntitlementCheck {
        feature: EntitlementCheckFeature {
            id: c.feature.id,
            code: c.feature.code,
        },
        allowed: c.allowed,
        has_access: c.allowed,
        reason,
        reason_type: c.reason_type.map(Into::into),
        remaining: limit.map(|l| (l - used.unwrap_or_default()).max(Decimal::ZERO)),
        used,
        limit,
        period,
        in_overage: c.levels.iter().any(|l| l.in_overage),
        levels: c
            .levels
            .into_iter()
            .map(|l| level_to_rest(l, codes))
            .collect(),
        hint: c.hint.map(|h| <&str>::from(h).to_owned()),
        attributes_ignored: c.attributes_ignored,
        warnings: c.warnings,
        exact: matches!(c.usage_source, DomainUsageSource::Read),
        usage_source: c.usage_source.into(),
        as_of: c.usage_as_of.unwrap_or(as_of).into(),
        lag_ms: 0,
        near_limit: c.near_limit,
        retry_after_ms,
        limits: vec![],
        value: c.value.and_then(config_of),
    }
}

fn snapshot_level_to_rest(l: DomainSnapshotLevel, codes: &TypeCodes) -> SnapshotLevel {
    let (value, reason) = match l.value {
        LevelValue::Value(v) => (Some(grant_value_to_rest(v)), None),
        LevelValue::Empty => (None, None),
        LevelValue::ResourceRequired => (None, Some(EntitlementCheckReason::ResourceRequired)),
        LevelValue::NotSubscribed => (None, Some(EntitlementCheckReason::NotSubscribed)),
    };
    SnapshotLevel {
        level: level_code(l.level, codes),
        path: l.path,
        value,
        reason,
        policy: l.policy.into(),
        used: l.used,
        reset_at: l.window.and_then(|w| w.end).map(Into::into),
        deciding_subscription_id: l.deciding_subscription,
        origin: l.origin.map(contribution_to_rest),
    }
}

/// The earlier list's value: the combined value with the own level's policy and usage.
/// What the earlier shapes can't hold: an app-counted limit, or no value at all.
fn absent_value(f: &SnapshotFeature) -> EffectiveEntitlementValue {
    match (&f.feature.kind, &f.value) {
        (domain::FeatureKind::Limit(_), value) => {
            EffectiveEntitlementValue::AppCounted(AppCountedEffectiveEntitlementValue {
                enabled: value
                    .as_ref()
                    .is_some_and(|v| *v != GrantValue::Limit(LimitValue::Off)),
                limit: match value {
                    Some(GrantValue::Limit(LimitValue::Amount(n))) => Some(*n),
                    _ => None,
                },
                used: f.levels.first().and_then(|l| l.used),
            })
        }
        _ => EffectiveEntitlementValue::NotEntitled(NotEntitledEffectiveEntitlementValue {}),
    }
}

fn legacy_value(f: &SnapshotFeature) -> Option<domain::EffectiveEntitlementValue> {
    use domain::{EffectiveEntitlementValue as E, ResolvedEntitlementValue as R};
    let own = f.levels.first()?;
    let value = f.value.clone().or(match f.feature.kind {
        domain::FeatureKind::Boolean => Some(GrantValue::Boolean(false)),
        domain::FeatureKind::Limit(_) => Some(GrantValue::Limit(LimitValue::Off)),
        domain::FeatureKind::Value(_) => None,
    })?;
    let subscribed = own.deciding_subscription.is_some();
    Some(
        match R::of(&f.feature.kind, &value, &own.policy, subscribed)? {
            R::Boolean { enabled } => E::Boolean { enabled },
            R::Config { value } => E::Config { value },
            R::Metered {
                metric_id,
                limit,
                reset_period,
                overage_behavior,
                thresholds,
                enabled,
                group_by,
            } => {
                let limit_reached = match (&overage_behavior, limit, own.used) {
                    (domain::OverageBehavior::Block { grace_period_pct }, Some(l), Some(used)) => {
                        enabled
                            && used * Decimal::ONE_HUNDRED
                                >= l * Decimal::from(100 + grace_period_pct.unwrap_or(0))
                    }
                    _ => false,
                };
                E::Metered {
                    metric_id,
                    limit,
                    reset_period,
                    overage_behavior,
                    thresholds,
                    enabled: enabled && !limit_reached,
                    group_by,
                    usage: domain::EntitlementUsage {
                        consumed: own.used,
                        remaining: limit
                            .map(|l| (l - own.used.unwrap_or_default()).max(Decimal::ZERO)),
                        period_start: own.window.as_ref().map(|w| w.start),
                        reset_at: own.window.as_ref().and_then(|w| w.end),
                        unavailable: own.usage_unavailable,
                    },
                    limit_reached,
                }
            }
        },
    )
}

pub fn snapshot_to_rest(s: Snapshot, codes: &TypeCodes) -> EntitlementSnapshot {
    EntitlementSnapshot {
        etag: s.etag,
        attributes_ignored: s.attributes_ignored,
        warnings: s.warnings,
        data: s
            .features
            .into_iter()
            .map(|f| SnapshotEntitlement {
                value: legacy_value(&f).map_or_else(|| absent_value(&f), effective_value_to_rest),
                feature: feature_ref(f.feature.to_ref()),
                scope: f.scope.into(),
                levels: f
                    .levels
                    .into_iter()
                    .map(|l| snapshot_level_to_rest(l, codes))
                    .collect(),
            })
            .collect(),
    }
}

impl From<AppCountResult> for ReportUsageResponse {
    fn from(r: AppCountResult) -> Self {
        Self {
            path: r.path,
            count: r.count,
            applied: r.applied,
            replayed: r.replayed,
        }
    }
}

/// The `<prefix>[<key>]=<value>` pairs of a query string, as `GET` passes `resources` and
/// `attributes`.
pub fn bracketed_params(query: Option<&str>, prefix: &str) -> BTreeMap<String, String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .filter_map(|(key, value)| {
            let name = key
                .strip_prefix(prefix)?
                .strip_prefix('[')?
                .strip_suffix(']')?;
            (!name.is_empty()).then(|| (name.to_owned(), value.into_owned()))
        })
        .collect()
}

pub fn resolved_entitlement_to_rest(r: domain::ResolvedEntitlement) -> ResolvedEntitlement {
    use domain::ResolvedEntitlementValue as DomVal;
    let value = match r.value {
        DomVal::Boolean { enabled } => {
            ResolvedEntitlementValue::Boolean(BooleanResolvedEntitlementValue { enabled })
        }
        DomVal::Metered {
            metric_id,
            limit,
            reset_period,
            overage_behavior,
            thresholds,
            enabled,
            group_by,
        } => ResolvedEntitlementValue::Metered(MeteredResolvedEntitlementValue {
            metric_id,
            limit,
            reset_period: reset_period.into(),
            enabled,
            overage_behavior: Some(overage_to_rest(&overage_behavior)),
            grace_pct: grace_of(&overage_behavior),
            warning_threshold_pct: thresholds.first().copied(),
            thresholds: non_empty(thresholds),
            group_by,
        }),
        DomVal::Config { value } => {
            ResolvedEntitlementValue::Config(ConfigResolvedEntitlementValue {
                value: value.into(),
            })
        }
    };
    ResolvedEntitlement {
        feature: feature_ref(r.feature),
        value,
    }
}

pub fn effective_entitlement_to_rest(r: domain::EffectiveEntitlement) -> EffectiveEntitlement {
    EffectiveEntitlement {
        feature: feature_ref(r.feature),
        value: effective_value_to_rest(r.value),
        own_grant: r.own_grant.map(|g| SubscriptionGrant {
            value: g.value.map(grant_value_to_rest),
            deciding_subscription_id: g.deciding_subscription_id,
            deciding_plan_name: g.deciding_plan_name,
            deciding_resource_path: g.deciding_resource_path,
        }),
    }
}

fn effective_value_to_rest(v: domain::EffectiveEntitlementValue) -> EffectiveEntitlementValue {
    match v {
        domain::EffectiveEntitlementValue::Boolean { enabled } => {
            EffectiveEntitlementValue::Boolean(BooleanEffectiveEntitlementValue { enabled })
        }
        domain::EffectiveEntitlementValue::Metered {
            metric_id,
            limit,
            reset_period,
            overage_behavior,
            thresholds,
            enabled,
            group_by,
            usage,
            limit_reached,
        } => EffectiveEntitlementValue::Metered(MeteredEffectiveEntitlementValue {
            spec: MeteredEntitlementSpec {
                metric_id,
                limit,
                reset_period: reset_period.into(),
                enabled,
                overage_behavior: Some(overage_to_rest(&overage_behavior)),
                grace_pct: grace_of(&overage_behavior),
                warning_threshold_pct: thresholds.first().copied(),
                thresholds: non_empty(thresholds),
                group_by,
            },
            usage: MeteredEntitlementUsage {
                consumed: usage.consumed,
                remaining: usage.remaining,
                reset_at: usage.reset_at.map(Into::into),
                limit_reached: Some(limit_reached),
            },
        }),
        domain::EffectiveEntitlementValue::Config { value } => {
            EffectiveEntitlementValue::Config(ConfigEffectiveEntitlementValue {
                value: value.into(),
            })
        }
    }
}

pub fn config_value_to_domain(v: ConfigValue) -> domain::ConfigValue {
    match v {
        ConfigValue::Number(n) => domain::ConfigValue::Number(n.value),
        ConfigValue::Boolean(b) => domain::ConfigValue::Boolean(b.value),
        ConfigValue::Text(t) => domain::ConfigValue::Text(t.value),
        ConfigValue::Json(j) => domain::ConfigValue::Json(j.value),
    }
}

fn overage_to_rest(overage: &domain::OverageBehavior) -> OverageBehavior {
    match overage {
        domain::OverageBehavior::Block { .. } => OverageBehavior::Block,
        domain::OverageBehavior::Allow => OverageBehavior::Allow,
    }
}

fn non_empty(thresholds: Vec<u32>) -> Option<Vec<u32>> {
    (!thresholds.is_empty()).then_some(thresholds)
}

fn grace_of(overage: &domain::OverageBehavior) -> Option<u32> {
    match overage {
        domain::OverageBehavior::Block { grace_period_pct } => *grace_period_pct,
        domain::OverageBehavior::Allow => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlements::model::EntitlementCheckReason as R;
    use meteroid_store::domain::entitlement_levels::CheckReason as C;

    #[test]
    fn a_denial_keeps_its_released_v1_name() {
        let off = GrantValue::Boolean(false);
        let limit_off = GrantValue::Limit(LimitValue::Off);
        let limit = GrantValue::Limit(LimitValue::Amount(Decimal::TEN));
        for (reason, value, expected, wire) in [
            (C::NotEntitled, None, R::NoEntitlement, "NO_ENTITLEMENT"),
            (C::NotEntitled, Some(&off), R::Disabled, "DISABLED"),
            (C::NotEntitled, Some(&limit_off), R::Disabled, "DISABLED"),
            (
                C::CreditExhausted,
                None,
                R::InsufficientBalance,
                "INSUFFICIENT_BALANCE",
            ),
            (
                C::LimitReached,
                Some(&limit),
                R::LimitReached,
                "LIMIT_REACHED",
            ),
            (
                C::BudgetExceeded,
                Some(&limit),
                R::BudgetExceeded,
                "BUDGET_EXCEEDED",
            ),
        ] {
            let named = check_reason(reason, value);
            assert_eq!(named, expected, "{reason:?} {value:?}");
            assert_eq!(serde_json::to_value(named).unwrap(), wire);
        }
    }
}
