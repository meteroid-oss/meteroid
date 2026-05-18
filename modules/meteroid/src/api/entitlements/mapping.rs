use common_domain::ids::{
    AddOnId, BillableMetricId, EntitlementEntityId, FeatureId, PlanId, PlanVersionId, ProductId,
    QuoteId, SubscriptionId,
};
use meteroid_grpc::meteroid::api::entitlements::v1 as proto;
use meteroid_store::domain::entitlements::{
    EffectiveEntitlementValue, EntitlementSpec, FeatureType, OverageBehavior, PeriodUnit,
    ResetPeriod, ResolvedEntitlement, ResolvedEntitlementValue, ResolvedOrigin,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::domain::{EffectiveEntitlement, Entitlement, EntitlementValue, Feature};
use rust_decimal::Decimal;

use crate::api::shared::conversions::ProtoConv;

pub fn calendar_unit_to_proto(v: &PeriodUnit) -> proto::CalendarUnit {
    match v {
        PeriodUnit::Hour => proto::CalendarUnit::Hour,
        PeriodUnit::Day => proto::CalendarUnit::Day,
        PeriodUnit::Week => proto::CalendarUnit::Week,
        PeriodUnit::Month => proto::CalendarUnit::Month,
        PeriodUnit::Year => proto::CalendarUnit::Year,
    }
}

pub fn calendar_unit_from_proto(v: proto::CalendarUnit) -> PeriodUnit {
    match v {
        proto::CalendarUnit::Hour => PeriodUnit::Hour,
        proto::CalendarUnit::Day => PeriodUnit::Day,
        proto::CalendarUnit::Week => PeriodUnit::Week,
        proto::CalendarUnit::Month => PeriodUnit::Month,
        proto::CalendarUnit::Year => PeriodUnit::Year,
    }
}

pub fn reset_period_to_proto(v: ResetPeriod) -> proto::ResetPeriod {
    use proto::reset_period::{BillingCycle, Calendar, FixedWindow, Inner, Never, SlidingWindow};
    proto::ResetPeriod {
        inner: Some(match v {
            ResetPeriod::BillingCycle => Inner::BillingCycle(BillingCycle {}),
            ResetPeriod::Calendar { unit, interval } => Inner::Calendar(Calendar {
                unit: calendar_unit_to_proto(&unit).into(),
                interval,
            }),
            ResetPeriod::FixedWindow { unit, interval } => Inner::FixedWindow(FixedWindow {
                unit: calendar_unit_to_proto(&unit).into(),
                interval,
            }),
            ResetPeriod::SlidingWindow { unit, interval } => Inner::SlidingWindow(SlidingWindow {
                unit: calendar_unit_to_proto(&unit).into(),
                interval,
            }),
            ResetPeriod::Never => Inner::Never(Never {}),
        }),
    }
}

pub fn reset_period_from_proto(v: proto::ResetPeriod) -> Result<ResetPeriod, tonic::Status> {
    use proto::reset_period::Inner;
    let unit_of = |u: i32| -> Result<PeriodUnit, tonic::Status> {
        Ok(calendar_unit_from_proto(
            proto::CalendarUnit::try_from(u)
                .map_err(|_| tonic::Status::invalid_argument("invalid calendar_unit"))?,
        ))
    };
    match v.inner {
        Some(Inner::BillingCycle(_)) => Ok(ResetPeriod::BillingCycle),
        Some(Inner::Calendar(c)) => Ok(ResetPeriod::Calendar {
            unit: unit_of(c.unit)?,
            interval: c.interval,
        }),
        Some(Inner::FixedWindow(r)) => Ok(ResetPeriod::FixedWindow {
            unit: unit_of(r.unit)?,
            interval: r.interval,
        }),
        Some(Inner::SlidingWindow(r)) => Ok(ResetPeriod::SlidingWindow {
            unit: unit_of(r.unit)?,
            interval: r.interval,
        }),
        Some(Inner::Never(_)) => Ok(ResetPeriod::Never),
        None => Err(tonic::Status::invalid_argument(
            "reset_period.inner is required",
        )),
    }
}

pub fn overage_behavior_to_proto(v: OverageBehavior) -> proto::OverageBehavior {
    use proto::overage_behavior::{Allow, Block, Inner};
    proto::OverageBehavior {
        inner: Some(match v {
            OverageBehavior::Block { grace_period_pct } => Inner::Block(Block { grace_period_pct }),
            OverageBehavior::Allow => Inner::Allow(Allow {}),
        }),
    }
}

pub fn overage_behavior_from_proto(
    v: Option<proto::OverageBehavior>,
) -> Result<OverageBehavior, tonic::Status> {
    use proto::overage_behavior::Inner;
    match v.and_then(|o| o.inner) {
        Some(Inner::Block(b)) => Ok(OverageBehavior::Block {
            grace_period_pct: b.grace_period_pct,
        }),
        Some(Inner::Allow(_)) => Ok(OverageBehavior::Allow),
        // Required on the wire; default to Block-no-grace if caller omits.
        None => Ok(OverageBehavior::Block {
            grace_period_pct: None,
        }),
    }
}

pub fn feature_status_to_proto(v: FeatureStatusEnum) -> i32 {
    match v {
        FeatureStatusEnum::Active => proto::FeatureStatus::Active as i32,
        FeatureStatusEnum::Disabled => proto::FeatureStatus::Disabled as i32,
        FeatureStatusEnum::Archived => proto::FeatureStatus::Archived as i32,
    }
}

pub fn feature_status_from_proto(v: i32) -> Result<FeatureStatusEnum, tonic::Status> {
    match proto::FeatureStatus::try_from(v)
        .map_err(|_| tonic::Status::invalid_argument("invalid feature_status"))?
    {
        proto::FeatureStatus::Active => Ok(FeatureStatusEnum::Active),
        proto::FeatureStatus::Disabled => Ok(FeatureStatusEnum::Disabled),
        proto::FeatureStatus::Archived => Ok(FeatureStatusEnum::Archived),
    }
}

pub fn feature_type_to_proto(v: FeatureType) -> proto::FeatureType {
    use proto::feature_type::{BooleanFeature, Inner, MeteredFeature};
    proto::FeatureType {
        inner: Some(match v {
            FeatureType::Boolean => Inner::Boolean(BooleanFeature {}),
            FeatureType::Metered { metric_id } => Inner::Metered(MeteredFeature {
                metric_id: metric_id.as_proto(),
            }),
        }),
    }
}

pub fn feature_type_from_proto(
    v: Option<proto::FeatureType>,
) -> Result<FeatureType, tonic::Status> {
    use proto::feature_type::Inner;
    match v.and_then(|ft| ft.inner) {
        Some(Inner::Boolean(_)) => Ok(FeatureType::Boolean),
        Some(Inner::Metered(m)) => Ok(FeatureType::Metered {
            metric_id: BillableMetricId::from_proto(&m.metric_id)?,
        }),
        None => Err(tonic::Status::invalid_argument("feature_type is required")),
    }
}

pub fn origin_to_proto(o: ResolvedOrigin) -> proto::ResolvedOrigin {
    proto::ResolvedOrigin {
        entity: Some(entity_to_proto(o.entity)),
        name: o.name,
    }
}

pub fn entity_to_proto(entity: EntitlementEntityId) -> proto::EntitlementEntity {
    use proto::entitlement_entity::EntityId;
    proto::EntitlementEntity {
        entity_id: Some(match entity {
            EntitlementEntityId::Feature(id) => EntityId::FeatureId(id.as_proto()),
            EntitlementEntityId::Plan(id) => EntityId::PlanId(id.as_proto()),
            EntitlementEntityId::PlanVersion(id) => EntityId::PlanVersionId(id.as_proto()),
            EntitlementEntityId::AddOn(id) => EntityId::AddOnId(id.as_proto()),
            EntitlementEntityId::Subscription(id) => EntityId::SubscriptionId(id.as_proto()),
            EntitlementEntityId::Quote(id) => EntityId::QuoteId(id.as_proto()),
        }),
    }
}

pub fn entity_from_proto(
    entity: Option<&proto::EntitlementEntity>,
) -> Result<EntitlementEntityId, tonic::Status> {
    use proto::entitlement_entity::EntityId;
    match entity.and_then(|e| e.entity_id.as_ref()) {
        Some(EntityId::FeatureId(id)) => {
            Ok(EntitlementEntityId::Feature(FeatureId::from_proto(id)?))
        }
        Some(EntityId::PlanId(id)) => Ok(EntitlementEntityId::Plan(PlanId::from_proto(id)?)),
        Some(EntityId::PlanVersionId(id)) => Ok(EntitlementEntityId::PlanVersion(
            PlanVersionId::from_proto(id)?,
        )),
        Some(EntityId::AddOnId(id)) => Ok(EntitlementEntityId::AddOn(AddOnId::from_proto(id)?)),
        Some(EntityId::SubscriptionId(id)) => Ok(EntitlementEntityId::Subscription(
            SubscriptionId::from_proto(id)?,
        )),
        Some(EntityId::QuoteId(id)) => Ok(EntitlementEntityId::Quote(QuoteId::from_proto(id)?)),
        None => Err(tonic::Status::invalid_argument("entity_id is required")),
    }
}

pub fn entitlement_value_from_proto(
    v: Option<proto::EntitlementValue>,
) -> Result<EntitlementValue, tonic::Status> {
    use proto::entitlement_value::Value;
    match v.and_then(|ev| ev.value) {
        Some(Value::BooleanValue(b)) => Ok(EntitlementValue::Boolean { enabled: b.enabled }),
        Some(Value::MeteredValue(m)) => {
            let reset_period = m
                .reset_period
                .map(reset_period_from_proto)
                .transpose()?
                .unwrap_or(ResetPeriod::Never);
            Ok(EntitlementValue::Metered {
                limit: m
                    .limit
                    .as_deref()
                    .map(|s| {
                        s.parse::<Decimal>()
                            .map_err(|_| tonic::Status::invalid_argument("invalid limit value"))
                    })
                    .transpose()?,
                reset_period,
                overage_behavior: overage_behavior_from_proto(m.overage_behavior)?,
                warning_threshold_pct: m.warning_threshold_pct,
                enabled: m.enabled,
            })
        }
        None => Err(tonic::Status::invalid_argument(
            "entitlement_value is required",
        )),
    }
}

pub fn feature_to_proto(f: Feature) -> proto::Feature {
    let product = f.product.map(|p| proto::ProductRef {
        id: p.id.as_proto(),
        name: p.name,
    });
    proto::Feature {
        id: f.id.as_proto(),
        name: f.name,
        description: f.description,
        feature_type: Some(feature_type_to_proto(f.feature_type)),
        status: feature_status_to_proto(f.status),
        product,
        created_at: f.created_at.as_proto(),
    }
}

pub fn build_value_proto(v: EntitlementValue) -> proto::EntitlementValue {
    use proto::entitlement_value::{BooleanValue, MeteredValue, Value};
    let value = match v {
        EntitlementValue::Boolean { enabled } => {
            Some(Value::BooleanValue(BooleanValue { enabled }))
        }
        EntitlementValue::Metered {
            limit,
            reset_period,
            overage_behavior,
            warning_threshold_pct,
            enabled,
        } => Some(Value::MeteredValue(MeteredValue {
            limit: limit.map(|d| d.to_string()),
            reset_period: Some(reset_period_to_proto(reset_period)),
            overage_behavior: Some(overage_behavior_to_proto(overage_behavior)),
            warning_threshold_pct,
            enabled,
        })),
    };
    proto::EntitlementValue { value }
}

pub fn entitlement_to_proto(e: Entitlement) -> proto::Entitlement {
    proto::Entitlement {
        id: e.id.as_proto(),
        feature_id: e.feature_id.as_proto(),
        entity: Some(entity_to_proto(e.entity)),
        value: Some(build_value_proto(e.value)),
        created_at: e.created_at.as_proto(),
        updated_at: e.updated_at.as_proto(),
    }
}

pub fn effective_entitlement_to_proto(r: EffectiveEntitlement) -> proto::EffectiveEntitlement {
    use proto::effective_entitlement::Value;
    let value = match r.value {
        EffectiveEntitlementValue::Boolean { enabled } => {
            Value::Boolean(proto::effective_entitlement::BooleanEntitlement { enabled })
        }
        EffectiveEntitlementValue::Metered {
            metric_id,
            limit,
            reset_period,
            overage_behavior,
            warning_threshold_pct,
            enabled,
            usage,
        } => Value::Metered(proto::effective_entitlement::MeteredEntitlement {
            metric_id: metric_id.as_proto(),
            limit: limit.map(|d| d.to_string()),
            consumed: usage.consumed.map(|d| d.to_string()),
            remaining: usage.remaining.map(|d| d.to_string()),
            reset_at: usage.reset_at.map(|t| t.as_proto()),
            reset_period: Some(reset_period_to_proto(reset_period)),
            overage_behavior: Some(overage_behavior_to_proto(overage_behavior)),
            warning_threshold_pct,
            enabled,
        }),
    };
    proto::EffectiveEntitlement {
        feature: Some(proto::FeatureRef {
            id: r.feature.id.as_proto(),
            name: r.feature.name,
            product: r.feature.product.map(|p| proto::ProductRef {
                id: p.id.as_proto(),
                name: p.name,
            }),
        }),
        value: Some(value),
        created_at: r.created_at.as_proto(),
        origin: Some(origin_to_proto(r.origin)),
    }
}

pub fn resolved_entitlement_to_proto(r: ResolvedEntitlement) -> proto::ResolvedEntitlement {
    use proto::resolved_entitlement::Value;
    let value = match r.value {
        ResolvedEntitlementValue::Boolean { enabled } => {
            Value::Boolean(proto::resolved_entitlement::BooleanResolved { enabled })
        }
        ResolvedEntitlementValue::Metered {
            metric_id,
            limit,
            reset_period,
            overage_behavior,
            warning_threshold_pct,
            enabled,
        } => Value::Metered(proto::resolved_entitlement::MeteredResolved {
            metric_id: metric_id.as_proto(),
            limit: limit.map(|d| d.to_string()),
            reset_period: Some(reset_period_to_proto(reset_period)),
            overage_behavior: Some(overage_behavior_to_proto(overage_behavior)),
            warning_threshold_pct,
            enabled,
        }),
    };
    proto::ResolvedEntitlement {
        feature: Some(proto::FeatureRef {
            id: r.feature.id.as_proto(),
            name: r.feature.name,
            product: r.feature.product.map(|p| proto::ProductRef {
                id: p.id.as_proto(),
                name: p.name,
            }),
        }),
        value: Some(value),
        created_at: r.created_at.as_proto(),
        origin: Some(origin_to_proto(r.origin)),
    }
}

pub fn entitlement_spec_from_proto(
    spec: proto::EntitlementSpec,
) -> Result<EntitlementSpec, tonic::Status> {
    Ok(EntitlementSpec {
        feature_id: FeatureId::from_proto(&spec.feature_id)?,
        value: entitlement_value_from_proto(spec.value)?,
    })
}

pub fn product_id_from_proto(s: Option<String>) -> Result<Option<ProductId>, tonic::Status> {
    s.map(|p| ProductId::from_proto(&p)).transpose()
}
