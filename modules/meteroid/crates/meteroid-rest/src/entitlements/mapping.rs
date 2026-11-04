use meteroid_store::domain::entitlements::{self as domain};

use super::model::*;

pub fn feature_to_rest(f: domain::Feature) -> Feature {
    Feature {
        id: f.id,
        name: f.name,
        code: f.code,
        description: f.description,
        feature_type: f.feature_type.into(),
        status: f.status.into(),
        product: f.product.map(|p| ProductRef {
            id: p.id,
            name: p.name,
        }),
        created_at: f.created_at.into(),
        entitlement: f.entitlement.map(entitlement_to_rest),
    }
}

pub fn value_to_rest(v: domain::EntitlementValue) -> EntitlementValue {
    match v {
        domain::EntitlementValue::Boolean { enabled } => {
            EntitlementValue::Boolean(BooleanEntitlementValue { enabled })
        }
        domain::EntitlementValue::Metered {
            limit,
            reset_period,
            overage_behavior,
            thresholds,
            enabled,
            group_by,
        } => EntitlementValue::Metered(MeteredEntitlementValue {
            limit,
            reset_period: reset_period.into(),
            enabled,
            overage_behavior: Some(Some(overage_to_rest(&overage_behavior))),
            grace_pct: grace_of(&overage_behavior).map(Some),
            warning_threshold_pct: thresholds.first().copied().map(Some),
            thresholds: non_empty(thresholds).map(Some),
            group_by: group_by.map(Some),
        }),
        domain::EntitlementValue::Config { value } => {
            EntitlementValue::Config(ConfigEntitlementValue {
                value: value.into(),
            })
        }
    }
}

pub fn entitlement_to_rest(e: domain::Entitlement) -> Entitlement {
    Entitlement {
        id: e.id,
        feature_id: e.feature_id,
        value: value_to_rest(e.value),
        created_at: e.created_at.into(),
        updated_at: e.updated_at.into(),
    }
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
        feature: FeatureRef {
            id: r.feature.id,
            name: r.feature.name,
            code: r.feature.code,
            product: r.feature.product.map(|p| ProductRef {
                id: p.id,
                name: p.name,
            }),
        },
        value,
    }
}

pub fn effective_entitlement_to_rest(r: domain::EffectiveEntitlement) -> EffectiveEntitlement {
    let value = match r.value {
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
    };
    EffectiveEntitlement {
        feature: FeatureRef {
            id: r.feature.id,
            name: r.feature.name,
            code: r.feature.code,
            product: r.feature.product.map(|p| ProductRef {
                id: p.id,
                name: p.name,
            }),
        },
        value,
    }
}

pub fn feature_type_to_domain(t: FeatureType) -> domain::FeatureType {
    match t {
        FeatureType::Boolean(_) => domain::FeatureType::Boolean,
        FeatureType::Metered(m) => domain::FeatureType::Metered {
            metric_id: m.metric_id,
        },
        FeatureType::Config(c) => domain::FeatureType::Config {
            value_type: match c.value_type {
                ConfigValueType::Number => domain::ConfigValueType::Number,
                ConfigValueType::Boolean => domain::ConfigValueType::Boolean,
                ConfigValueType::Text => domain::ConfigValueType::Text,
                ConfigValueType::Map => domain::ConfigValueType::Map,
                ConfigValueType::Json => domain::ConfigValueType::Json,
                ConfigValueType::Select => domain::ConfigValueType::Select { options: c.options },
            },
        },
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

/// The limit terms of a metered value. An omitted term is taken from `base` (the defaults on
/// create, the stored value on update), unless it no longer applies to the new limit or window:
/// then it falls back to its default. A term sent explicitly is kept, for the store to refuse.
fn metered_to_domain(
    m: MeteredEntitlementValue,
    base: MeteredBase,
) -> Result<domain::EntitlementValue, String> {
    let has_limit = m.limit.is_some();
    let sliding = matches!(m.reset_period, ResetPeriod::SlidingWindow(_));
    let overage = match m.overage_behavior {
        Some(explicit) => explicit.unwrap_or(OverageBehavior::Block),
        None if has_limit => base.overage,
        None => OverageBehavior::Block,
    };
    let grace = match m.grace_pct {
        Some(explicit) => explicit,
        None if has_limit && overage == OverageBehavior::Block => base.grace_pct,
        None => None,
    };
    if overage == OverageBehavior::Allow && matches!(m.grace_pct, Some(Some(_))) {
        return Err("grace_pct applies to the BLOCK overage behavior only".to_owned());
    }
    // `thresholds` wins over the deprecated single `warning_threshold_pct`.
    let explicit = match (m.thresholds, m.warning_threshold_pct) {
        (Some(thresholds), _) => Some(thresholds.unwrap_or_default()),
        (None, Some(warning)) => Some(warning.into_iter().collect()),
        (None, None) => None,
    };
    let thresholds = match explicit {
        Some(explicit) => domain::requested_thresholds(explicit, None),
        None if has_limit && !sliding => base.thresholds,
        None => Vec::new(),
    };
    Ok(domain::EntitlementValue::Metered {
        limit: m.limit,
        reset_period: m.reset_period.into(),
        overage_behavior: match overage {
            OverageBehavior::Block => domain::OverageBehavior::Block {
                grace_period_pct: grace,
            },
            OverageBehavior::Allow => domain::OverageBehavior::Allow,
        },
        thresholds,
        enabled: m.enabled,
        group_by: m.group_by.unwrap_or(base.group_by),
    })
}

struct MeteredBase {
    overage: OverageBehavior,
    grace_pct: Option<u32>,
    thresholds: Vec<u32>,
    group_by: Option<String>,
}

impl MeteredBase {
    fn defaults() -> Self {
        Self {
            overage: OverageBehavior::Block,
            grace_pct: None,
            thresholds: Vec::new(),
            group_by: None,
        }
    }
}

pub fn value_to_domain(v: EntitlementValue) -> Result<domain::EntitlementValue, String> {
    Ok(match v {
        EntitlementValue::Boolean(b) => domain::EntitlementValue::Boolean { enabled: b.enabled },
        EntitlementValue::Metered(m) => metered_to_domain(m, MeteredBase::defaults())?,
        EntitlementValue::Config(c) => domain::EntitlementValue::Config {
            value: config_value_to_domain(c.value),
        },
    })
}

/// Update variant of `value_to_domain`: the store replaces the whole value, so the limit terms
/// the body omits are carried over from the stored value.
pub fn value_update_to_domain(
    v: EntitlementValue,
    existing: &domain::EntitlementValue,
) -> Result<domain::EntitlementValue, String> {
    match (v, existing) {
        (
            EntitlementValue::Metered(m),
            domain::EntitlementValue::Metered {
                overage_behavior,
                thresholds,
                group_by,
                ..
            },
        ) => metered_to_domain(
            m,
            MeteredBase {
                overage: overage_to_rest(overage_behavior),
                grace_pct: grace_of(overage_behavior),
                thresholds: thresholds.clone(),
                group_by: group_by.clone(),
            },
        ),
        (value, _) => value_to_domain(value),
    }
}

pub fn entitlement_spec_to_domain(
    spec: EntitlementSpecRequest,
) -> Result<domain::EntitlementSpec, String> {
    Ok(domain::EntitlementSpec {
        feature_id: spec.feature_id,
        value: value_to_domain(spec.value)?,
    })
}
