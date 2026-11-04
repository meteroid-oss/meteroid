use common_domain::ids::{
    AddOnId, BillableMetricId, CustomerId, EntitlementEntityId, FeatureId, PlanId, PlanVersionId,
    ProductId, QuoteId, SubscriptionId,
};
use meteroid_grpc::meteroid::api::entitlements::v1 as proto;
use meteroid_store::domain::entitlements::{
    ConfigValue, ConfigValueType, EffectiveEntitlementValue, EntitlementSpec, FeatureType,
    OverageBehavior, PeriodUnit, ResetPeriod, ResolvedEntitlement, ResolvedEntitlementValue,
    ResolvedOrigin, requested_thresholds,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::domain::{EffectiveEntitlement, Entitlement, EntitlementValue, Feature};
use meteroid_store::services::CreditPaidFeature;
use rust_decimal::Decimal;

use meteroid_app::conversions::ProtoConv;

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
    match v.map(|o| o.inner) {
        None => Ok(OverageBehavior::Block {
            grace_period_pct: None,
        }),
        Some(Some(Inner::Block(b))) => Ok(OverageBehavior::Block {
            grace_period_pct: b.grace_period_pct,
        }),
        Some(Some(Inner::Allow(_))) => Ok(OverageBehavior::Allow),
        Some(None) => Err(tonic::Status::invalid_argument(
            "overage_behavior.inner is required",
        )),
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

pub fn config_value_to_proto(v: ConfigValue) -> proto::ConfigValue {
    use proto::config_value::Value;
    proto::ConfigValue {
        value: Some(match v {
            ConfigValue::Number(d) => Value::Number(d.to_string()),
            ConfigValue::Boolean(b) => Value::Boolean(b),
            ConfigValue::Text(s) => Value::Text(s),
            ConfigValue::Json(j) => Value::Json(j.to_string()),
        }),
    }
}

pub fn config_value_from_proto(
    v: Option<proto::ConfigValue>,
) -> Result<ConfigValue, tonic::Status> {
    use proto::config_value::Value;
    match v.and_then(|c| c.value) {
        Some(Value::Number(s)) => {
            Ok(ConfigValue::Number(s.parse::<Decimal>().map_err(|_| {
                tonic::Status::invalid_argument("invalid config number value")
            })?))
        }
        Some(Value::Boolean(b)) => Ok(ConfigValue::Boolean(b)),
        Some(Value::Text(s)) => Ok(ConfigValue::Text(s)),
        Some(Value::Json(s)) => Ok(ConfigValue::Json(
            serde_json::from_str(&s)
                .map_err(|_| tonic::Status::invalid_argument("invalid config json value"))?,
        )),
        None => Err(tonic::Status::invalid_argument("config value is required")),
    }
}

pub fn feature_type_to_proto(v: FeatureType) -> proto::FeatureType {
    use proto::feature_type::{BooleanFeature, ConfigFeature, Inner, MeteredFeature};
    proto::FeatureType {
        inner: Some(match v {
            FeatureType::Boolean => Inner::Boolean(BooleanFeature {}),
            FeatureType::Metered { metric_id } => Inner::Metered(MeteredFeature {
                metric_id: metric_id.as_proto(),
            }),
            FeatureType::Config { value_type } => {
                let (vt, options) = config_value_type_to_proto(value_type);
                Inner::Config(ConfigFeature {
                    value_type: vt as i32,
                    options,
                })
            }
        }),
    }
}

fn config_value_type_to_proto(v: ConfigValueType) -> (proto::ConfigValueType, Vec<String>) {
    match v {
        ConfigValueType::Number => (proto::ConfigValueType::Number, vec![]),
        ConfigValueType::Boolean => (proto::ConfigValueType::Boolean, vec![]),
        ConfigValueType::Text => (proto::ConfigValueType::Text, vec![]),
        ConfigValueType::Map => (proto::ConfigValueType::Map, vec![]),
        ConfigValueType::Json => (proto::ConfigValueType::Json, vec![]),
        ConfigValueType::Select { options } => (proto::ConfigValueType::Select, options),
    }
}

fn config_value_type_from_proto(
    v: i32,
    options: Vec<String>,
) -> Result<ConfigValueType, tonic::Status> {
    match proto::ConfigValueType::try_from(v).unwrap_or(proto::ConfigValueType::Unspecified) {
        proto::ConfigValueType::Number => Ok(ConfigValueType::Number),
        proto::ConfigValueType::Boolean => Ok(ConfigValueType::Boolean),
        proto::ConfigValueType::Text => Ok(ConfigValueType::Text),
        proto::ConfigValueType::Map => Ok(ConfigValueType::Map),
        proto::ConfigValueType::Json => Ok(ConfigValueType::Json),
        proto::ConfigValueType::Select => Ok(ConfigValueType::Select { options }),
        proto::ConfigValueType::Unspecified => Err(tonic::Status::invalid_argument(
            "config value_type is required",
        )),
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
        Some(Inner::Config(c)) => Ok(FeatureType::Config {
            value_type: config_value_type_from_proto(c.value_type, c.options)?,
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
            EntitlementEntityId::Customer(id) => EntityId::CustomerId(id.as_proto()),
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
        Some(EntityId::CustomerId(id)) => {
            Ok(EntitlementEntityId::Customer(CustomerId::from_proto(id)?))
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
                thresholds: requested_thresholds(m.thresholds, m.warning_threshold_pct),
                enabled: m.enabled,
                group_by: m.group_by,
            })
        }
        Some(Value::ConfigValue(c)) => Ok(EntitlementValue::Config {
            value: config_value_from_proto(Some(c))?,
        }),
        None => Err(tonic::Status::invalid_argument(
            "entitlement_value is required",
        )),
    }
}

pub fn feature_ref_to_proto(
    f: meteroid_store::domain::entitlements::FeatureRef,
) -> proto::FeatureRef {
    proto::FeatureRef {
        id: f.id.as_proto(),
        name: f.name,
        code: f.code,
        product: f.product.map(|p| proto::ProductRef {
            id: p.id.as_proto(),
            name: p.name,
        }),
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
        code: f.code,
        entitlement: f.entitlement.map(entitlement_to_proto),
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
            thresholds,
            enabled,
            group_by,
        } => Some(Value::MeteredValue(MeteredValue {
            limit: limit.map(|d| d.to_string()),
            reset_period: Some(reset_period_to_proto(reset_period)),
            enabled,
            group_by,
            overage_behavior: Some(overage_behavior_to_proto(overage_behavior)),
            warning_threshold_pct: thresholds.first().copied(),
            thresholds,
        })),
        EntitlementValue::Config { value } => {
            Some(Value::ConfigValue(config_value_to_proto(value)))
        }
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

pub fn credit_paid_feature_to_proto(feature: CreditPaidFeature) -> proto::CreditPaidFeature {
    proto::CreditPaidFeature {
        feature_id: feature.feature_id.as_proto(),
        metric_id: feature.metric_id.as_proto(),
        payers: feature
            .payers
            .into_iter()
            .map(|payer| proto::CreditPayer {
                wallet_id: payer.wallet_id.as_proto(),
                subscription_id: payer.subscription_id.as_proto(),
                product_id: payer.product_id.map(|id| id.as_proto()),
            })
            .collect(),
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
            thresholds,
            enabled,
            group_by,
            usage,
            limit_reached,
        } => Value::Metered(proto::effective_entitlement::MeteredEntitlement {
            metric_id: metric_id.as_proto(),
            limit: limit.map(|d| d.to_string()),
            consumed: usage.consumed.map(|d| d.to_string()),
            remaining: usage.remaining.map(|d| d.to_string()),
            reset_at: usage.reset_at.map(|t| t.as_proto()),
            reset_period: Some(reset_period_to_proto(reset_period)),
            enabled,
            group_by,
            overage_behavior: Some(overage_behavior_to_proto(overage_behavior)),
            warning_threshold_pct: thresholds.first().copied(),
            thresholds,
            limit_reached,
        }),
        EffectiveEntitlementValue::Config { value } => {
            Value::Config(proto::effective_entitlement::ConfigEntitlement {
                value: Some(config_value_to_proto(value)),
            })
        }
    };
    proto::EffectiveEntitlement {
        feature: Some(proto::FeatureRef {
            id: r.feature.id.as_proto(),
            name: r.feature.name,
            code: r.feature.code,
            product: r.feature.product.map(|p| proto::ProductRef {
                id: p.id.as_proto(),
                name: p.name,
            }),
        }),
        value: Some(value),
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
            thresholds,
            enabled,
            group_by,
        } => Value::Metered(proto::resolved_entitlement::MeteredResolved {
            metric_id: metric_id.as_proto(),
            limit: limit.map(|d| d.to_string()),
            reset_period: Some(reset_period_to_proto(reset_period)),
            enabled,
            group_by,
            overage_behavior: Some(overage_behavior_to_proto(overage_behavior)),
            warning_threshold_pct: thresholds.first().copied(),
            thresholds,
        }),
        ResolvedEntitlementValue::Config { value } => {
            Value::Config(proto::resolved_entitlement::ConfigResolved {
                value: Some(config_value_to_proto(value)),
            })
        }
    };
    proto::ResolvedEntitlement {
        feature: Some(proto::FeatureRef {
            id: r.feature.id.as_proto(),
            name: r.feature.name,
            code: r.feature.code,
            product: r.feature.product.map(|p| proto::ProductRef {
                id: p.id.as_proto(),
                name: p.name,
            }),
        }),
        value: Some(value),
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
