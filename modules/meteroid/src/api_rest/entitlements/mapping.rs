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
        created_at: f.created_at,
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
            overage_behavior: _,
            warning_threshold_pct: _,
            enabled,
        } => EntitlementValue::Metered(MeteredEntitlementValue {
            limit,
            reset_period: reset_period.into(),
            enabled,
        }),
    }
}

pub fn entitlement_to_rest(e: domain::Entitlement) -> Entitlement {
    Entitlement {
        id: e.id,
        feature_id: e.feature_id,
        value: value_to_rest(e.value),
        created_at: e.created_at,
        updated_at: e.updated_at,
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
            overage_behavior: _,
            warning_threshold_pct: _,
            enabled,
        } => ResolvedEntitlementValue::Metered(MeteredResolvedEntitlementValue {
            metric_id,
            limit,
            reset_period: reset_period.into(),
            enabled,
        }),
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
            overage_behavior: _,
            warning_threshold_pct: _,
            enabled,
            usage,
        } => EffectiveEntitlementValue::Metered(MeteredEffectiveEntitlementValue {
            spec: MeteredEntitlementSpec {
                metric_id,
                limit,
                reset_period: reset_period.into(),
                enabled,
            },
            usage: MeteredEntitlementUsage {
                consumed: usage.consumed,
                remaining: usage.remaining,
                reset_at: usage.reset_at,
            },
        }),
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
