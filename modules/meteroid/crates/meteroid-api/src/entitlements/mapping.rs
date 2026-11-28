use chrono::{DateTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, AliasOr, BillableMetricId, CustomerId, EntitlementEntityId, FeatureId, PlanId,
    PlanVersionId, ProductId, QuoteId, ResourceTypeId, SubscriptionId,
};
use meteroid_grpc::meteroid::api::entitlements::v1 as proto;
use meteroid_store::domain::entitlement_checks::{
    AppCountExplain, CheckLevel, CheckLevelExplain, CheckRequest, CheckResponse, DecidingCandidate,
    FeatureScope, LevelValue, ResourceSnapshot, Snapshot, SnapshotFeature, SnapshotLevel,
    SnapshotRead, UsageSource,
};
use meteroid_store::domain::entitlement_fold::{
    Condition, Contribution, Discarded, Effect, GrantValue, LimitValue, RowLevel, Tier,
};
use meteroid_store::domain::entitlement_levels::{CheckReason, ReasonType};
use meteroid_store::domain::entitlement_policy::{AtLimit, FeaturePolicy, Period, Resets};
use meteroid_store::domain::entitlements::{
    ConfigValue, EffectiveEntitlementValue, EntitlementSpec, EntitlementUpdate, FeatureKind,
    FeatureProductRef, LimitCountedBy, OverageBehavior, PeriodUnit, ResetPeriod,
    ResolvedEntitlement, ResolvedEntitlementValue, ResolvedOrigin, SubscriptionGrant, ValueType,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::domain::{EffectiveEntitlement, Entitlement, Feature};
use meteroid_store::services::CreditPaidFeature;
use meteroid_store::services::entitlements::app_counts::{
    AppCountOp, AppCountResult, AppCountWrite,
};
use rust_decimal::Decimal;
use tonic::Status;

use meteroid_app::conversions::ProtoConv;

fn invalid(what: &str) -> Status {
    Status::invalid_argument(format!("invalid {what}"))
}

fn time(s: Option<String>) -> Result<Option<DateTime<Utc>>, Status> {
    s.as_ref().map(DateTime::<Utc>::from_proto_ref).transpose()
}

fn decimal(s: Option<String>) -> Result<Option<Decimal>, Status> {
    s.as_ref().map(Decimal::from_proto_ref).transpose()
}

/// `clear` wins: `Some(None)` removes the value, `None` keeps it.
fn patch<T>(value: Option<T>, clear: bool) -> Option<Option<T>> {
    if clear { Some(None) } else { value.map(Some) }
}

fn feature_ref(feature: &str) -> AliasOr<FeatureId> {
    feature.parse().unwrap_or_else(|never| match never {})
}

pub fn grant_value_to_proto(v: GrantValue) -> proto::EntitlementValue {
    use proto::entitlement_value::{BooleanValue, MeteredValue, Value};
    let config = |c: ConfigValue| Value::ConfigValue(config_value_to_proto(c));
    let value = match v {
        GrantValue::Boolean(enabled) => Value::BooleanValue(BooleanValue { enabled }),
        GrantValue::Limit(limit) => Value::MeteredValue(MeteredValue {
            limit: match limit {
                LimitValue::Amount(n) => Some(n.as_proto()),
                LimitValue::Off | LimitValue::Unlimited => None,
            },
            enabled: limit != LimitValue::Off,
        }),
        GrantValue::Number(n) => config(ConfigValue::Number(n)),
        GrantValue::Text(t) => config(ConfigValue::Text(t)),
        GrantValue::Json(j) => config(ConfigValue::Json(j)),
    };
    proto::EntitlementValue { value: Some(value) }
}

pub fn product_ref_to_proto(p: FeatureProductRef) -> proto::ProductRef {
    proto::ProductRef {
        id: p.id.as_proto(),
        name: p.name,
    }
}

pub fn subscription_grant_to_proto(g: SubscriptionGrant) -> proto::SubscriptionGrant {
    proto::SubscriptionGrant {
        value: g.value.map(grant_value_to_proto),
        deciding_subscription_id: g.deciding_subscription_id.as_proto(),
        deciding_plan_name: g.deciding_plan_name,
        deciding_resource_path: g.deciding_resource_path,
    }
}

pub fn grant_value_from_proto(v: Option<proto::EntitlementValue>) -> Result<GrantValue, Status> {
    use proto::entitlement_value::Value;
    let value = v
        .and_then(|v| v.value)
        .ok_or_else(|| Status::invalid_argument("entitlement value is required"))?;
    Ok(match value {
        Value::BooleanValue(b) => GrantValue::Boolean(b.enabled),
        Value::MeteredValue(m) if !m.enabled => GrantValue::Limit(LimitValue::Off),
        Value::MeteredValue(m) => {
            GrantValue::Limit(decimal(m.limit)?.map_or(LimitValue::Unlimited, LimitValue::Amount))
        }
        Value::ConfigValue(c) => match config_value_from_proto(Some(c))? {
            ConfigValue::Boolean(b) => GrantValue::Boolean(b),
            ConfigValue::Number(n) => GrantValue::Number(n),
            ConfigValue::Text(t) => GrantValue::Text(t),
            ConfigValue::Json(j) => GrantValue::Json(j),
        },
    })
}

pub fn policy_to_proto(p: FeaturePolicy) -> proto::FeaturePolicy {
    use proto::{PolicyAtLimit as A, PolicyResets as R};
    proto::FeaturePolicy {
        resets: match p.resets {
            Resets::BillingCycle => R::BillingCycle,
            Resets::Calendar => R::Calendar,
            Resets::Fixed => R::Fixed,
            Resets::Sliding => R::Sliding,
            Resets::Never => R::Never,
        } as i32,
        period: p.period.map(|period| proto::PolicyPeriod {
            unit: calendar_unit_to_proto(&period.unit).into(),
            interval: period.interval,
        }),
        at_limit: match p.at_limit {
            AtLimit::Deny => A::Deny,
            AtLimit::AllowAndBill => A::AllowAndBill,
            AtLimit::DenyNew => A::DenyNew,
        } as i32,
        grace_pct: p.grace_pct,
        thresholds: p.thresholds,
    }
}

pub fn policy_from_proto(p: proto::FeaturePolicy) -> Result<FeaturePolicy, Status> {
    use proto::{PolicyAtLimit as A, PolicyResets as R};
    Ok(FeaturePolicy {
        resets: match R::try_from(p.resets) {
            Ok(R::BillingCycle) => Resets::BillingCycle,
            Ok(R::Calendar) => Resets::Calendar,
            Ok(R::Fixed) => Resets::Fixed,
            Ok(R::Sliding) => Resets::Sliding,
            Ok(R::Never) => Resets::Never,
            Ok(R::Unspecified) | Err(_) => return Err(invalid("policy.resets")),
        },
        period: p
            .period
            .map(|period| {
                Ok::<_, Status>(Period {
                    unit: calendar_unit_from_proto(period.unit)?,
                    interval: period.interval,
                })
            })
            .transpose()?,
        at_limit: match A::try_from(p.at_limit) {
            Ok(A::Deny) => AtLimit::Deny,
            Ok(A::AllowAndBill) => AtLimit::AllowAndBill,
            Ok(A::DenyNew) => AtLimit::DenyNew,
            Ok(A::Unspecified) | Err(_) => return Err(invalid("policy.at_limit")),
        },
        grace_pct: p.grace_pct,
        thresholds: p.thresholds,
    })
}

pub(crate) fn effect_to_proto(e: Effect) -> i32 {
    use proto::EntitlementEffect as E;
    (match e {
        Effect::Set => E::Set,
        Effect::Add => E::Add,
        Effect::RaiseTo => E::RaiseTo,
        Effect::CapAt => E::CapAt,
    }) as i32
}

/// `None`: the source's default effect.
pub(crate) fn effect_from_proto(v: i32) -> Result<Option<Effect>, Status> {
    use proto::EntitlementEffect as E;
    match E::try_from(v).map_err(|_| invalid("effect"))? {
        E::Unspecified => Ok(None),
        E::Set => Ok(Some(Effect::Set)),
        E::Add => Ok(Some(Effect::Add)),
        E::RaiseTo => Ok(Some(Effect::RaiseTo)),
        E::CapAt => Ok(Some(Effect::CapAt)),
    }
}

fn row_level_to_proto(level: RowLevel) -> (i32, Option<String>) {
    use proto::EntitlementLevel as L;
    match level {
        RowLevel::Feature => (L::Feature as i32, None),
        RowLevel::Customer => (L::Customer as i32, None),
        RowLevel::Type(id) => (L::Type as i32, Some(id.as_proto())),
    }
}

fn row_level_from_proto(level: i32, type_id: Option<String>) -> Result<RowLevel, Status> {
    use proto::EntitlementLevel as L;
    match (L::try_from(level), type_id) {
        (Ok(L::Feature), None) => Ok(RowLevel::Feature),
        (Ok(L::Customer), None) => Ok(RowLevel::Customer),
        (Ok(L::Type), Some(id)) => Ok(RowLevel::Type(ResourceTypeId::from_proto(id)?)),
        _ => Err(Status::invalid_argument(
            "level_type_id is required by the TYPE level, and refused by the others",
        )),
    }
}

fn condition_to_proto(c: Condition) -> proto::Condition {
    proto::Condition {
        clauses: c
            .0
            .into_iter()
            .map(|(attribute, values)| (attribute, proto::ConditionValues { values }))
            .collect(),
    }
}

fn condition_from_proto(c: proto::Condition) -> Condition {
    Condition(c.clauses.into_iter().map(|(a, v)| (a, v.values)).collect())
}

pub fn actor_to_proto(actor: Actor) -> proto::Actor {
    let (kind, id) = match actor {
        Actor::System => ("SYSTEM", None),
        Actor::User { id } => ("USER", Some(id.as_proto())),
        Actor::ApiToken { id } => ("API_KEY", Some(id.as_proto())),
        Actor::Customer { id } => ("CUSTOMER", Some(id.as_proto())),
        Actor::OAuthApp { id } => ("OAUTH_APP", Some(id.as_proto())),
        Actor::Workflow { id } => ("WORKFLOW", Some(id.as_proto())),
        Actor::QuoteRecipient { email } => ("QUOTE_RECIPIENT", Some(email)),
    };
    proto::Actor {
        r#type: kind.to_string(),
        id,
    }
}

type KindProto = (
    proto::FeatureKind,
    Option<proto::FeatureValueSpec>,
    Option<proto::FeatureLimitSpec>,
);

fn kind_to_proto(kind: FeatureKind) -> KindProto {
    use proto::FeatureValueType as T;
    let value = |r#type: T, higher_is_better, options, ordered| proto::FeatureValueSpec {
        r#type: r#type as i32,
        higher_is_better,
        options,
        ordered,
    };
    match kind {
        FeatureKind::Boolean => (proto::FeatureKind::Boolean, None, None),
        FeatureKind::Value(v) => {
            let spec = match v {
                ValueType::Number { higher_is_better } => {
                    value(T::Number, Some(higher_is_better), vec![], false)
                }
                ValueType::Text => value(T::Text, None, vec![], false),
                ValueType::Select { options, ordered } => value(T::Select, None, options, ordered),
                ValueType::Json => value(T::Json, None, vec![], false),
            };
            (proto::FeatureKind::Value, Some(spec), None)
        }
        FeatureKind::Limit(counted_by) => {
            let (counted_by, metric_id) = match counted_by {
                LimitCountedBy::Events(metric_id) => {
                    (proto::LimitCountedBy::Events, Some(metric_id.as_proto()))
                }
                LimitCountedBy::App => (proto::LimitCountedBy::App, None),
            };
            let spec = proto::FeatureLimitSpec {
                counted_by: counted_by as i32,
                metric_id,
            };
            (proto::FeatureKind::Limit, None, Some(spec))
        }
    }
}

pub fn kind_from_proto(
    kind: i32,
    value: Option<proto::FeatureValueSpec>,
    limit: Option<proto::FeatureLimitSpec>,
) -> Result<FeatureKind, Status> {
    use proto::FeatureValueType as T;
    match (proto::FeatureKind::try_from(kind), value, limit) {
        (Ok(proto::FeatureKind::Boolean), None, None) => Ok(FeatureKind::Boolean),
        (Ok(proto::FeatureKind::Value), Some(v), None) => {
            Ok(FeatureKind::Value(match T::try_from(v.r#type) {
                Ok(T::Number) => ValueType::Number {
                    higher_is_better: v.higher_is_better.unwrap_or(true),
                },
                Ok(T::Text) => ValueType::Text,
                Ok(T::Select) => ValueType::Select {
                    options: v.options,
                    ordered: v.ordered,
                },
                Ok(T::Json) => ValueType::Json,
                Ok(T::Unspecified) | Err(_) => return Err(invalid("value.type")),
            }))
        }
        (Ok(proto::FeatureKind::Limit), None, Some(l)) => Ok(FeatureKind::Limit(
            match proto::LimitCountedBy::try_from(l.counted_by) {
                Ok(proto::LimitCountedBy::Events) => {
                    LimitCountedBy::Events(BillableMetricId::from_proto(
                        l.metric_id
                            .ok_or_else(|| invalid("limit: EVENTS requires metric_id"))?,
                    )?)
                }
                Ok(proto::LimitCountedBy::App) if l.metric_id.is_none() => LimitCountedBy::App,
                _ => return Err(invalid("limit.counted_by")),
            },
        )),
        _ => Err(Status::invalid_argument(
            "kind is required: VALUE takes `value`, LIMIT takes `limit`, BOOLEAN neither",
        )),
    }
}

pub fn feature_to_proto(f: Feature) -> proto::Feature {
    let feature_type = feature_type_to_proto(&f.kind);
    let (kind, value, limit) = kind_to_proto(f.kind);
    let defaults: Vec<_> = f.defaults.into_iter().map(entitlement_to_proto).collect();
    proto::Feature {
        id: f.id.as_proto(),
        name: f.name,
        description: f.description,
        feature_type,
        status: feature_status_to_proto(f.status),
        product: f.product.map(product_ref_to_proto),
        created_at: f.created_at.as_proto(),
        code: f.code,
        entitlement: defaults.first().cloned(),
        defaults,
        kind: kind as i32,
        value,
        limit,
        unit: f.unit,
        level_resource_type_id: f.level_type_id.map(|id| id.as_proto()),
        policy: f.policy.map(policy_to_proto),
    }
}

pub fn entitlement_to_proto(e: Entitlement) -> proto::Entitlement {
    let (level, level_type_id) = row_level_to_proto(e.level);
    proto::Entitlement {
        id: e.id.as_proto(),
        feature_id: e.feature_id.as_proto(),
        entity: Some(entity_to_proto(e.entity)),
        value: Some(grant_value_to_proto(e.value)),
        created_at: e.lineage_created_at.as_proto(),
        updated_at: e.created_at.as_proto(),
        effect: effect_to_proto(e.effect),
        level,
        level_type_id,
        resource_id: e.resource_id.map(|id| id.as_proto()),
        condition: e.condition.map(condition_to_proto),
        valid_from: e.valid_from.map(|t| t.as_proto()),
        valid_until: e.valid_until.map(|t| t.as_proto()),
        reason: e.reason,
        policy_override: e.policy_override.map(policy_to_proto),
        created_by: e.created_by.map(actor_to_proto),
        revoked_at: e.revoked_at.map(|t| t.as_proto()),
        revoked_by: e.revoked_by.map(|id| id.to_string()),
    }
}

pub fn entitlement_spec_from_proto(
    spec: proto::EntitlementSpec,
) -> Result<EntitlementSpec, Status> {
    Ok(EntitlementSpec {
        feature_id: FeatureId::from_proto(&spec.feature_id)?,
        value: grant_value_from_proto(spec.value)?,
        effect: effect_from_proto(spec.effect)?,
        level: row_level_from_proto(spec.level, spec.level_type_id)?,
        condition: spec.condition.map(condition_from_proto),
        valid_from: time(spec.valid_from)?,
        valid_until: time(spec.valid_until)?,
        reason: spec.reason,
        policy_override: spec.policy_override.map(policy_from_proto).transpose()?,
    })
}

pub fn entitlement_update_from_proto(
    r: proto::UpdateEntitlementRequest,
) -> Result<EntitlementUpdate, Status> {
    Ok(EntitlementUpdate {
        value: r
            .value
            .map(|v| grant_value_from_proto(Some(v)))
            .transpose()?,
        effect: effect_from_proto(r.effect)?,
        condition: patch(r.condition.map(condition_from_proto), r.clear_condition),
        valid_from: patch(time(r.valid_from)?, r.clear_valid_from),
        valid_until: patch(time(r.valid_until)?, r.clear_valid_until),
        reason: patch(r.reason, r.clear_reason),
        policy_override: patch(
            r.policy_override.map(policy_from_proto).transpose()?,
            r.clear_policy_override,
        ),
    })
}

pub fn check_request_from_proto(
    r: proto::CheckEntitlementRequest,
    actor: &Actor,
) -> Result<(CustomerId, AliasOr<FeatureId>, CheckRequest), Status> {
    Ok((
        CustomerId::from_proto(&r.customer_id)?,
        feature_ref(&r.feature),
        CheckRequest {
            resources: r.resources.into_iter().collect(),
            attributes: r.attributes.into_iter().collect(),
            requested: decimal(r.requested)?,
            current_count: decimal(r.current_count)?,
            explain: r.explain,
            subscription_id: SubscriptionId::from_proto_opt(r.subscription_id)?,
            api_key_id: match actor {
                Actor::ApiToken { id } => Some(*id),
                _ => None,
            },
        },
    ))
}

fn reason_to_proto(reason: CheckReason) -> i32 {
    use proto::CheckReason as R;
    (match reason {
        CheckReason::ResourceRequired => R::ResourceRequired,
        CheckReason::MetricNotFound => R::MetricNotFound,
        CheckReason::NotSubscribed => R::NotSubscribed,
        CheckReason::NotEntitled => R::NotEntitled,
        CheckReason::UsageUnavailable => R::UsageUnavailable,
        CheckReason::LimitReached => R::LimitReached,
        CheckReason::BudgetExceeded => R::BudgetExceeded,
        CheckReason::CreditExhausted => R::CreditExhausted,
    }) as i32
}

fn reason_type_to_proto(reason_type: ReasonType) -> i32 {
    (match reason_type {
        ReasonType::Denied => proto::CheckReasonType::Denied,
        ReasonType::Misconfigured => proto::CheckReasonType::Misconfigured,
        ReasonType::Unavailable => proto::CheckReasonType::Unavailable,
    }) as i32
}

fn tier_to_proto(tier: Tier) -> i32 {
    use proto::Tier as T;
    (match tier {
        Tier::Default => T::Default,
        Tier::Plan => T::Plan,
        Tier::PlanVersion => T::PlanVersion,
        Tier::Subscription => T::Subscription,
        Tier::AddOn => T::AddOn,
        Tier::Customer => T::Customer,
        Tier::ResourceOverride => T::ResourceOverride,
    }) as i32
}

fn discarded_to_proto(discarded: Option<Discarded>) -> i32 {
    use proto::DiscardReason as D;
    (match discarded {
        None => D::Unspecified,
        Some(Discarded::ReplacedByHigherTier) => D::ReplacedByHigherTier,
        Some(Discarded::LostWithinTier) => D::LostWithinTier,
        Some(Discarded::NoEffect) => D::NoEffect,
        Some(Discarded::NoBaseOnLevel) => D::NoBaseOnLevel,
        Some(Discarded::EffectNotAllowed) => D::EffectNotAllowed,
        Some(Discarded::ConditionNotMet) => D::ConditionNotMet,
        Some(Discarded::NotInEffect) => D::NotInEffect,
    }) as i32
}

fn contribution_to_proto(c: Contribution) -> proto::Contribution {
    proto::Contribution {
        entitlement_id: c.lineage_id.as_proto(),
        source: Some(entity_to_proto(c.source)),
        source_name: c.source_name,
        tier: tier_to_proto(c.tier),
        effect: effect_to_proto(c.effect),
        value: Some(grant_value_to_proto(c.value)),
        condition: c.condition.map(condition_to_proto),
        quantity: c.quantity,
        units: c.units,
        policy_override: c.policy_override,
        running: c.running.map(grant_value_to_proto),
        discarded: discarded_to_proto(c.discarded),
    }
}

fn app_count_explain_to_proto(a: AppCountExplain) -> proto::AppCountExplain {
    proto::AppCountExplain {
        path: a.path,
        count: a.count.as_proto(),
        last_set_value: a.last_set_value.map(|d| d.as_proto()),
        last_set_at: a.last_set_at.map(|t| t.as_proto()),
        last_set_key: a.last_set_key,
        adds_since: a.adds_since,
        adds_sum: a.adds_sum.as_proto(),
        last_add_at: a.last_add_at.map(|t| t.as_proto()),
    }
}

fn candidate_to_proto(c: DecidingCandidate) -> proto::DecidingCandidate {
    proto::DecidingCandidate {
        subscription_id: c.subscription_id.as_proto(),
        resource_path: c.resource_path,
        value: c.value.map(grant_value_to_proto),
    }
}

fn level_explain_to_proto(e: CheckLevelExplain) -> proto::LevelExplain {
    proto::LevelExplain {
        contributions: e
            .contributions
            .into_iter()
            .map(contribution_to_proto)
            .collect(),
        candidates: e.candidates.into_iter().map(candidate_to_proto).collect(),
        app_counts: e
            .app_counts
            .into_iter()
            .map(app_count_explain_to_proto)
            .collect(),
    }
}

pub fn level_to_proto(l: CheckLevel) -> proto::EntitlementLevelState {
    proto::EntitlementLevelState {
        resource_type_id: l.level.type_id().map(|id| id.as_proto()),
        path: l.path,
        value: l.value.map(grant_value_to_proto),
        used: l.used.map(|d| d.as_proto()),
        limit: l.limit.map(|d| d.as_proto()),
        near_limit: l.near_limit,
        reset_at: l.reset_at.map(|t| t.as_proto()),
        deciding_subscription_id: l.deciding_subscription.map(|id| id.as_proto()),
        allowed: l.denial.is_none(),
        reason: l.denial.map(reason_to_proto),
        explain: l.explain.map(level_explain_to_proto),
        in_overage: l.in_overage,
    }
}

pub fn check_response_to_proto(c: CheckResponse) -> proto::CheckEntitlementResponse {
    proto::CheckEntitlementResponse {
        feature: Some(proto::CheckedFeature {
            id: c.feature.id.as_proto(),
            code: c.feature.code,
        }),
        allowed: c.allowed,
        reason: c.reason.map(reason_to_proto),
        reason_type: c.reason_type.map(reason_type_to_proto),
        in_overage: c.levels.iter().any(|l| l.in_overage),
        levels: c.levels.into_iter().map(level_to_proto).collect(),
        hint: c.hint.map(|h| <&str>::from(h).to_owned()),
        attributes_ignored: c.attributes_ignored,
        warnings: c.warnings,
        usage_source: match c.usage_source {
            UsageSource::Read => proto::UsageSource::Read,
            UsageSource::Monitor => proto::UsageSource::Monitor,
        } as i32,
        near_limit: c.near_limit,
        retry_after: c.retry_after.map(|t| t.as_proto()),
        value: c.value.map(grant_value_to_proto),
        usage_as_of: c.usage_as_of.map(|t| t.as_proto()),
    }
}

fn snapshot_level_to_proto(l: SnapshotLevel) -> proto::SnapshotLevel {
    let (value, reason) = match l.value {
        LevelValue::Value(v) => (Some(grant_value_to_proto(v)), None),
        LevelValue::Empty => (None, None),
        LevelValue::ResourceRequired => {
            (None, Some(reason_to_proto(CheckReason::ResourceRequired)))
        }
        LevelValue::NotSubscribed => (None, Some(reason_to_proto(CheckReason::NotSubscribed))),
    };
    proto::SnapshotLevel {
        resource_type_id: l.level.type_id().map(|id| id.as_proto()),
        path: l.path,
        value,
        reason,
        policy: Some(policy_to_proto(l.policy)),
        used: l.used.map(|d| d.as_proto()),
        reset_at: l.window.and_then(|w| w.end).map(|t| t.as_proto()),
        deciding_subscription_id: l.deciding_subscription.map(|id| id.as_proto()),
        origin: l.origin.map(contribution_to_proto),
        usage_unavailable: l.usage_unavailable,
    }
}

fn snapshot_feature_to_proto(f: SnapshotFeature) -> proto::SnapshotFeature {
    proto::SnapshotFeature {
        feature: Some(feature_ref_to_proto(f.feature.to_ref())),
        scope: match f.scope {
            FeatureScope::Customer => proto::FeatureScope::Customer,
            FeatureScope::Resource => proto::FeatureScope::Resource,
        } as i32,
        value: f.value.map(grant_value_to_proto),
        levels: f.levels.into_iter().map(snapshot_level_to_proto).collect(),
    }
}

pub fn snapshot_to_proto(read: SnapshotRead) -> proto::GetEntitlementSnapshotResponse {
    match read {
        SnapshotRead::NotModified { etag } => proto::GetEntitlementSnapshotResponse {
            etag,
            not_modified: true,
            ..Default::default()
        },
        SnapshotRead::Modified(Snapshot {
            etag,
            features,
            attributes_ignored,
            warnings,
        }) => proto::GetEntitlementSnapshotResponse {
            etag,
            not_modified: false,
            features: features
                .into_iter()
                .map(snapshot_feature_to_proto)
                .collect(),
            attributes_ignored,
            warnings,
        },
    }
}

pub fn snapshots_to_proto(
    snapshots: Vec<ResourceSnapshot>,
) -> proto::GetEntitlementSnapshotsResponse {
    proto::GetEntitlementSnapshotsResponse {
        snapshots: snapshots
            .into_iter()
            .map(|s| proto::ResourceSnapshot {
                resource_id: s.resource_id.as_proto(),
                features: s
                    .features
                    .into_iter()
                    .map(snapshot_feature_to_proto)
                    .collect(),
                warnings: s.warnings,
            })
            .collect(),
    }
}

pub fn app_count_write_from_proto(
    r: proto::ReportEntitlementUsageRequest,
) -> Result<(CustomerId, AliasOr<FeatureId>, AppCountWrite), Status> {
    use proto::report_entitlement_usage_request::Op;
    let op = match r.op {
        Some(Op::Set(n)) => AppCountOp::Set(Decimal::from_proto_ref(&n)?),
        Some(Op::Add(n)) => AppCountOp::Add(Decimal::from_proto_ref(&n)?),
        None => {
            return Err(Status::invalid_argument(
                "one of `set` or `add` is required",
            ));
        }
    };
    if r.idempotency_key.trim().is_empty() {
        return Err(Status::invalid_argument("idempotency_key is required"));
    }
    Ok((
        CustomerId::from_proto(&r.customer_id)?,
        feature_ref(&r.feature),
        AppCountWrite {
            resources: r.resources.into_iter().collect(),
            op,
            at: time(r.at)?,
            idempotency_key: r.idempotency_key,
        },
    ))
}

pub fn app_count_result_to_proto(r: AppCountResult) -> proto::ReportEntitlementUsageResponse {
    proto::ReportEntitlementUsageResponse {
        path: r.path,
        count: r.count.as_proto(),
        applied: r.applied,
        replayed: r.replayed,
    }
}

fn calendar_unit_from_proto(v: i32) -> Result<PeriodUnit, Status> {
    use proto::CalendarUnit as U;
    Ok(
        match U::try_from(v).map_err(|_| invalid("policy.period.unit"))? {
            U::Hour => PeriodUnit::Hour,
            U::Day => PeriodUnit::Day,
            U::Week => PeriodUnit::Week,
            U::Month => PeriodUnit::Month,
            U::Year => PeriodUnit::Year,
        },
    )
}

pub fn calendar_unit_to_proto(v: &PeriodUnit) -> proto::CalendarUnit {
    match v {
        PeriodUnit::Hour => proto::CalendarUnit::Hour,
        PeriodUnit::Day => proto::CalendarUnit::Day,
        PeriodUnit::Week => proto::CalendarUnit::Week,
        PeriodUnit::Month => proto::CalendarUnit::Month,
        PeriodUnit::Year => proto::CalendarUnit::Year,
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

pub fn overage_behavior_to_proto(v: OverageBehavior) -> proto::OverageBehavior {
    use proto::overage_behavior::{Allow, Block, Inner};
    proto::OverageBehavior {
        inner: Some(match v {
            OverageBehavior::Block { grace_period_pct } => Inner::Block(Block { grace_period_pct }),
            OverageBehavior::Allow => Inner::Allow(Allow {}),
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

/// The read-only legacy type; `None` for a limit counted by the app, which has no metric.
fn feature_type_to_proto(kind: &FeatureKind) -> Option<proto::FeatureType> {
    use proto::ConfigValueType as C;
    use proto::feature_type::{BooleanFeature, ConfigFeature, Inner, MeteredFeature};
    let config = |value_type: C, options: &[String]| {
        Inner::Config(ConfigFeature {
            value_type: value_type as i32,
            options: options.to_vec(),
        })
    };
    let inner = match kind {
        FeatureKind::Boolean => Inner::Boolean(BooleanFeature {}),
        FeatureKind::Limit(LimitCountedBy::Events(metric_id)) => Inner::Metered(MeteredFeature {
            metric_id: metric_id.as_proto(),
        }),
        FeatureKind::Limit(LimitCountedBy::App) => return None,
        FeatureKind::Value(ValueType::Number { .. }) => config(C::Number, &[]),
        FeatureKind::Value(ValueType::Text) => config(C::Text, &[]),
        FeatureKind::Value(ValueType::Select { options, .. }) => config(C::Select, options),
        FeatureKind::Value(ValueType::Json) => config(C::Json, &[]),
    };
    Some(proto::FeatureType { inner: Some(inner) })
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

pub fn feature_ref_to_proto(
    f: meteroid_store::domain::entitlements::FeatureRef,
) -> proto::FeatureRef {
    proto::FeatureRef {
        id: f.id.as_proto(),
        name: f.name,
        code: f.code,
        product: f.product.map(product_ref_to_proto),
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
            usage_unavailable: usage.unavailable,
        }),
        EffectiveEntitlementValue::Config { value } => {
            Value::Config(proto::effective_entitlement::ConfigEntitlement {
                value: Some(config_value_to_proto(value)),
            })
        }
    };
    proto::EffectiveEntitlement {
        feature: Some(feature_ref_to_proto(r.feature)),
        value: Some(value),
        origin: Some(origin_to_proto(r.origin)),
        own_grant: r.own_grant.map(subscription_grant_to_proto),
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
        feature: Some(feature_ref_to_proto(r.feature)),
        value: Some(value),
        origin: Some(origin_to_proto(r.origin)),
        own_grant: None,
    }
}

pub fn product_id_from_proto(s: Option<String>) -> Result<Option<ProductId>, tonic::Status> {
    s.map(|p| ProductId::from_proto(&p)).transpose()
}
