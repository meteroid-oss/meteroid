pub mod app_counts;
pub(crate) mod check;
pub(crate) mod limits;
pub(crate) mod reads;
pub(crate) mod resolve;
pub(crate) mod snapshot;
pub mod subscription;
pub(crate) mod tiers;

use crate::StoreResult;
use crate::domain::BillableMetric;
use crate::domain::entitlement_checks::{LevelUsage, LevelValue, ResolvedFeature, ResolvedLevel};
use crate::domain::entitlement_fold::{Contribution, EvalLevel, GrantValue, LimitValue};
use crate::domain::entitlements::{
    EffectiveEntitlement, EffectiveEntitlementValue, EntitlementUsage, Feature, FeatureKind,
    FeatureRef, LimitCountedBy, OverageBehavior, ResetPeriod, ResolvedEntitlement,
    ResolvedEntitlementValue, ResolvedOrigin, SubscriptionGrant,
};
use crate::hooks::ExpectedMonitor;
use crate::repositories::entitlements::with_origin_names;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::repositories::subscriptions::usage_attribution::usage_scopes_of;
use crate::services::Services;
use crate::services::entitlements::limits::rules::hard_limit_reached;
use crate::services::entitlements::limits::{level_monitor_id, load_metrics};
use crate::services::entitlements::resolve::UsageReading;
use crate::services::entitlements::snapshot::LevelRead;
use crate::services::entitlements::subscription::{IncludedLevel, SubscriptionOf, included_levels};
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::ids::{
    BillableMetricId, CustomerId, EntitlementEntityId, FeatureId, ResourceId, SubscriptionId,
    TenantId,
};
use futures::stream::{self, StreamExt, TryStreamExt};
use rust_decimal::Decimal;
use std::collections::HashMap;
use uuid::Uuid;

/// Cap on concurrent metering-backend fetches per readout.
pub const USAGE_FETCH_CONCURRENCY: usize = 8;

struct MeteredMeta {
    feature: FeatureRef,
    origin: ResolvedOrigin,
    metric_id: BillableMetricId,
    limit: Option<Decimal>,
    reset_period: ResetPeriod,
    overage_behavior: OverageBehavior,
    thresholds: Vec<u32>,
    enabled: bool,
    period_start: NaiveDateTime,
    reset_at: Option<DateTime<Utc>>,
    group_by: Option<String>,
}

/// A subscription's current cycle, as `limited_usage_for_subscription` reads it.
#[derive(Clone, Copy, Debug)]
pub struct SubscriptionCycle {
    pub subscription_id: SubscriptionId,
    pub customer_id: CustomerId,
    pub resource_id: Option<ResourceId>,
    pub period_start: NaiveDate,
    /// Exclusive, as `subscription.current_period_end`.
    pub period_end: Option<NaiveDate>,
    pub activated_at: Option<NaiveDateTime>,
}

/// An enabled metered entitlement with a positive limit and its usage over its window.
#[derive(Clone, Debug)]
pub struct LimitedUsage {
    pub feature: FeatureRef,
    /// The level's: `''` for the customer, else its resource's (`site=s1`).
    pub path: String,
    pub metric_id: BillableMetricId,
    /// The deciding subscription's (§3.7).
    pub limit: Decimal,
    pub consumed: Decimal,
    pub reset_period: ResetPeriod,
    pub overage_behavior: OverageBehavior,
    pub thresholds: Vec<u32>,
    pub period_start: NaiveDateTime,
    /// Exclusive; `None` when the window never resets, or slides with `now`.
    pub period_end: Option<NaiveDateTime>,
    /// The limit applies to each value of this property; `consumed` is the largest group's.
    pub group_by: Option<String>,
    /// Another subscription decides the limit: what this one grants.
    pub own_grant: Option<SubscriptionGrant>,
}

impl LimitedUsage {
    /// The lowest alert threshold, the one that warns first.
    pub fn warning_pct(&self) -> Option<u32> {
        self.thresholds.first().copied()
    }

    /// Usage is refused: the limit blocks and its ceiling (limit plus grace) is reached.
    pub fn refused(&self) -> bool {
        limit_reached(&self.overage_behavior, Some(self.limit), self.consumed)
    }
}

fn finite_limit(level: &ResolvedLevel) -> Option<Decimal> {
    match level.value {
        LevelValue::Value(GrantValue::Limit(LimitValue::Amount(limit))) => Some(limit),
        _ => None,
    }
}

impl Services {
    /// Usage against every limit counted from events the subscription includes at `now`, each at
    /// its level's deciding limit and window, counting the level's usage (§3.7). Empty once the
    /// cycle has ended and the renewal is not processed yet: that period is no longer current.
    pub(crate) async fn limited_usage_for_subscription(
        &self,
        tenant_id: TenantId,
        sub: &SubscriptionCycle,
        now: NaiveDateTime,
        concurrency: usize,
    ) -> StoreResult<Vec<LimitedUsage>> {
        Ok(self
            .limited_usages(tenant_id, sub, now, concurrency, None)
            .await?
            .into_iter()
            .map(|(usage, _)| usage)
            .collect())
    }

    /// [`Self::limited_usage_for_subscription`] for a readout: each read as a snapshot reads it,
    /// with what it gave; `consumed` is 0 where it gave none.
    pub(crate) async fn limited_usage_readout(
        &self,
        tenant_id: TenantId,
        sub: &SubscriptionCycle,
        now: NaiveDateTime,
        concurrency: usize,
    ) -> StoreResult<Vec<(LimitedUsage, LevelUsage)>> {
        let reading = UsageReading::readout();
        self.limited_usages(tenant_id, sub, now, concurrency, Some(&reading))
            .await
    }

    /// Without `reading`, metering is read with the client's deadline and its failure fails all.
    async fn limited_usages(
        &self,
        tenant_id: TenantId,
        sub: &SubscriptionCycle,
        now: NaiveDateTime,
        concurrency: usize,
        reading: Option<&UsageReading>,
    ) -> StoreResult<Vec<(LimitedUsage, LevelUsage)>> {
        if sub
            .period_end
            .is_some_and(|end| now >= end.and_time(NaiveTime::MIN))
        {
            return Ok(vec![]);
        }
        let included = {
            let mut conn = self.store.get_conn().await?;
            let of = SubscriptionOf {
                id: sub.subscription_id,
                customer_id: sub.customer_id,
                resource_id: sub.resource_id,
            };
            included_levels(&mut conn, tenant_id, of, now.and_utc()).await?
        };
        let limited: Vec<(LimitedUsage, IncludedLevel)> = included
            .into_iter()
            .filter_map(|included| {
                let FeatureKind::Limit(LimitCountedBy::Events(metric_id)) =
                    included.feature.feature.kind
                else {
                    return None;
                };
                let limit = finite_limit(&included.level).filter(|l| *l > Decimal::ZERO)?;
                let window = included.level.window.as_ref()?;
                if included.level.level != EvalLevel::Customer && included.resource.is_none() {
                    return None;
                }
                let usage = LimitedUsage {
                    feature: included.feature.feature.to_ref(),
                    path: included.level.path.clone(),
                    metric_id,
                    limit,
                    consumed: Decimal::ZERO,
                    reset_period: window.reset_period.clone(),
                    overage_behavior: included.level.policy.overage_behavior(),
                    thresholds: included.level.policy.thresholds.clone(),
                    period_start: window.start.naive_utc(),
                    period_end: resets_at(&window.reset_period, window.end.map(|e| e.naive_utc())),
                    group_by: None,
                    own_grant: included.own_grant.clone(),
                };
                Some((usage, included))
            })
            .collect();

        let expected = limited
            .iter()
            .filter(|(_, included)| included.level.level == EvalLevel::Customer)
            .filter_map(|(usage, _)| {
                engine_monitor(
                    sub.customer_id,
                    usage.feature.id,
                    &usage.reset_period,
                    usage.period_start,
                    usage.limit,
                )
            })
            .collect::<Vec<_>>();
        let engine = self
            .engine_usage(tenant_id, sub.customer_id, &expected, now)
            .await;

        // Futures collected first: a stream `.map` closure borrowing the levels makes the
        // caller's future fail the higher-ranked `Send` check when spawned.
        let fetches: Vec<_> = limited
            .into_iter()
            .map(|(usage, included)| {
                let engine_consumed = (included.level.level == EvalLevel::Customer)
                    .then(|| {
                        engine.get(&level_monitor_id(
                            usage.feature.id,
                            sub.customer_id,
                            EvalLevel::Customer,
                        ))
                    })
                    .flatten()
                    .copied();
                async move {
                    if let Some(consumed) = engine_consumed {
                        return Ok((
                            LimitedUsage { consumed, ..usage },
                            LevelUsage::Read(Some(consumed)),
                        ));
                    }
                    let (feature, level) = (&included.feature.feature, &included.level);
                    let resource = included.resource.as_ref();
                    let read = match reading {
                        Some(reading) => {
                            self.store
                                .level_usage_read(
                                    tenant_id,
                                    sub.customer_id,
                                    feature,
                                    level,
                                    resource,
                                    now.and_utc(),
                                    None,
                                    reading,
                                )
                                .await?
                        }
                        None => LevelUsage::Read(
                            self.store
                                .level_usage(
                                    tenant_id,
                                    sub.customer_id,
                                    feature,
                                    level,
                                    resource,
                                    now.and_utc(),
                                )
                                .await?,
                        ),
                    };
                    let consumed = read.used().unwrap_or(Decimal::ZERO);
                    Ok((LimitedUsage { consumed, ..usage }, read))
                }
            })
            .collect();
        stream::iter(fetches)
            .buffered(concurrency.max(1))
            .try_collect()
            .await
    }

    pub(crate) async fn get_effective_entitlements(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        self.effective_entitlements(customer_id, tenant_id, None)
            .await
    }

    pub(crate) async fn get_effective_entitlement_for_feature(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
        feature_id: FeatureId,
    ) -> StoreResult<Option<EffectiveEntitlement>> {
        Ok(self
            .effective_entitlements(
                customer_id,
                tenant_id,
                Some(std::slice::from_ref(&feature_id)),
            )
            .await?
            .pop())
    }

    /// The customer-wide readout without a resource: each feature's own level, with its read
    /// usage over the deciding subscription's window.
    async fn effective_entitlements(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
        features: Option<&[FeatureId]>,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        let now = common_clock::now();
        let resolved = {
            let mut conn = self.store.get_conn().await?;
            self.store
                .resolve_customer_entitlements(
                    &mut conn,
                    tenant_id,
                    customer_id,
                    None,
                    features,
                    now,
                )
                .await?
        };
        let included = resolved
            .features
            .into_iter()
            .filter_map(|feature| {
                let level = feature.levels.iter().find(|l| l.own)?.clone();
                Some(IncludedLevel {
                    feature,
                    level,
                    resource: None,
                    own_grant: None,
                })
            })
            .collect();
        self.effective_of(tenant_id, customer_id, included, now)
            .await
    }

    /// A subscription's readout: each feature it includes at the deciding value of its level,
    /// with the level's usage, and its own grant when another subscription decides.
    pub(crate) async fn get_effective_entitlements_for_subscription(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        let sub = self
            .store
            .get_subscription(tenant_id, subscription_id)
            .await?;
        let now = common_clock::now();
        let included = {
            let mut conn = self.store.get_conn().await?;
            let of = SubscriptionOf {
                id: subscription_id,
                customer_id: sub.customer_id,
                resource_id: sub.resource_id,
            };
            included_levels(&mut conn, tenant_id, of, now).await?
        };
        self.effective_of(tenant_id, sub.customer_id, included, now)
            .await
    }

    /// Each level's usage read as a snapshot reads it: metering not answering in time leaves that
    /// entitlement's usage absent, the others answer.
    async fn effective_of(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        included: Vec<IncludedLevel>,
        now: DateTime<Utc>,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        let (included, readouts): (Vec<_>, Vec<_>) = included
            .into_iter()
            .filter_map(|i| {
                let readout = readout_at(&i.feature.feature, &i.level)?;
                Some((i, readout))
            })
            .unzip();
        let named = {
            let mut conn = self.store.get_conn().await?;
            with_origin_names(&mut conn, tenant_id, readouts).await?
        };
        let reads: Vec<LevelRead> = included
            .iter()
            .map(|i| (&i.feature.feature, &i.level, i.resource.as_ref()))
            .collect();
        let usages = self
            .store
            .read_usages(tenant_id, customer_id, &reads, now, UsageReading::readout())
            .await?;
        let mut result = Vec::with_capacity(named.len());
        for ((included, ent), usage) in included.into_iter().zip(named).zip(usages) {
            let value = match ent.value {
                ResolvedEntitlementValue::Boolean { enabled } => {
                    EffectiveEntitlementValue::Boolean { enabled }
                }
                ResolvedEntitlementValue::Config { value } => {
                    EffectiveEntitlementValue::Config { value }
                }
                ResolvedEntitlementValue::Metered {
                    metric_id,
                    limit,
                    reset_period,
                    overage_behavior,
                    thresholds,
                    enabled,
                    group_by,
                } => {
                    let window = included.level.window.as_ref();
                    let meta = MeteredMeta {
                        feature: ent.feature,
                        origin: ent.origin,
                        metric_id,
                        limit,
                        reset_at: window
                            .and_then(|w| resets_at(&reset_period, w.end.map(|e| e.naive_utc())))
                            .map(|t| t.and_utc()),
                        reset_period,
                        overage_behavior,
                        thresholds,
                        enabled,
                        period_start: window.map_or(now, |w| w.start).naive_utc(),
                        group_by,
                    };
                    let mut entitlement = match usage {
                        LevelUsage::Read(Some(consumed)) => {
                            build_metered_entitlement(meta, consumed)
                        }
                        unread => build_unread_metered_entitlement(
                            meta,
                            unread == LevelUsage::Unavailable,
                        ),
                    };
                    entitlement.own_grant = included.own_grant;
                    result.push(entitlement);
                    continue;
                }
            };
            result.push(EffectiveEntitlement {
                feature: ent.feature,
                value,
                origin: ent.origin,
                own_grant: included.own_grant,
            });
        }
        Ok(result)
    }

    /// Usage the usage controls' monitors hold for entitlement monitors, by monitor id. A missing
    /// or unreadable state is not an error: callers read the metering backend for what it lacks.
    async fn engine_usage(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        expected: &[ExpectedMonitor],
        now: NaiveDateTime,
    ) -> HashMap<Uuid, Decimal> {
        if expected.is_empty() {
            return HashMap::new();
        }
        let read = async {
            let mut conn = self.store.get_conn().await?;
            self.store
                .hooks
                .credit
                .entitlement_monitor_usage(
                    &mut conn,
                    tenant_id,
                    customer_id,
                    expected,
                    now.and_utc(),
                )
                .await
        };
        read.await.unwrap_or_else(|error| {
            log::warn!("engine usage unreadable for customer {customer_id}: {error:?}");
            HashMap::new()
        })
    }

    /// The metrics, measured as `subscription_id`'s usage filter attributes its usage when the
    /// entitlements are that subscription's.
    pub(crate) async fn load_metrics(
        &self,
        conn: &mut PgConn,
        metric_ids: &[BillableMetricId],
        tenant_id: &TenantId,
        subscription_id: Option<SubscriptionId>,
    ) -> StoreResult<HashMap<BillableMetricId, BillableMetric>> {
        let metrics = load_metrics(conn, metric_ids, *tenant_id).await?;
        let Some(subscription_id) = subscription_id else {
            return Ok(metrics);
        };
        let scopes = usage_scopes_of(conn, *tenant_id, subscription_id).await?;
        Ok(metrics
            .into_iter()
            .map(|(id, metric)| (id, scopes.apply(metric)))
            .collect())
    }
}

impl crate::Store {
    /// The subscription's readout without usage, for the gRPC subscription entitlements.
    pub async fn subscription_entitlements(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Vec<(ResolvedEntitlement, Option<SubscriptionGrant>)>> {
        let sub = self.get_subscription(tenant_id, subscription_id).await?;
        let mut conn = self.get_conn().await?;
        let of = SubscriptionOf {
            id: subscription_id,
            customer_id: sub.customer_id,
            resource_id: sub.resource_id,
        };
        let (grants, readouts): (Vec<_>, Vec<_>) =
            included_levels(&mut conn, tenant_id, of, common_clock::now())
                .await?
                .into_iter()
                .filter_map(|i| Some((i.own_grant, readout_at(&i.feature.feature, &i.level)?)))
                .unzip();
        Ok(with_origin_names(&mut conn, tenant_id, readouts)
            .await?
            .into_iter()
            .zip(grants)
            .collect())
    }
}

/// A resolved feature as the legacy readout shows it: its own level's value, with the source
/// of the row that set it. `None` when that level has no value or holds an app count.
pub(crate) fn readout_of(
    resolved: &ResolvedFeature,
) -> Option<(FeatureRef, ResolvedEntitlementValue, EntitlementEntityId)> {
    readout_at(&resolved.feature, resolved.levels.iter().find(|l| l.own)?)
}

/// A level's value as the legacy readout shows it, with the source of the row that set it.
pub(crate) fn readout_at(
    feature: &Feature,
    level: &ResolvedLevel,
) -> Option<(FeatureRef, ResolvedEntitlementValue, EntitlementEntityId)> {
    let LevelValue::Value(value) = &level.value else {
        return None;
    };
    let readout = ResolvedEntitlementValue::of(
        &feature.kind,
        value,
        &level.policy,
        level.deciding_subscription.is_some(),
    )?;
    let origin =
        winning(&level.explain).map_or(EntitlementEntityId::Feature(feature.id), |c| c.source);
    Some((feature.to_ref(), readout, origin))
}

/// The row that set a level's value: its last applied one.
pub(crate) fn winning(explain: &[Contribution]) -> Option<&Contribution> {
    explain.iter().rev().find(|c| c.running.is_some())
}

/// The monitor the engine keeps for a limited entitlement window; none for a sliding window,
/// which it does not monitor.
fn engine_monitor(
    customer_id: CustomerId,
    feature_id: FeatureId,
    reset_period: &ResetPeriod,
    period_start: NaiveDateTime,
    limit: Decimal,
) -> Option<ExpectedMonitor> {
    if matches!(reset_period, ResetPeriod::SlidingWindow { .. }) {
        return None;
    }
    Some(ExpectedMonitor {
        id: level_monitor_id(feature_id, customer_id, EvalLevel::Customer),
        window_start: period_start.and_utc(),
        limit,
        grouped: false,
    })
}

/// When the window's usage resets; a sliding window has no such moment, its usage leaves it
/// gradually.
fn resets_at(reset_period: &ResetPeriod, end: Option<NaiveDateTime>) -> Option<NaiveDateTime> {
    match reset_period {
        ResetPeriod::SlidingWindow { .. } => None,
        _ => end,
    }
}

/// A blocking limit refuses once usage reaches it plus its grace; an allowing one never does.
pub(crate) fn limit_reached(
    overage_behavior: &OverageBehavior,
    limit: Option<Decimal>,
    consumed: Decimal,
) -> bool {
    match (overage_behavior, limit) {
        (OverageBehavior::Block { grace_period_pct }, Some(limit)) => {
            hard_limit_reached(consumed, limit, *grace_period_pct)
        }
        (OverageBehavior::Allow, _) | (_, None) => false,
    }
}

fn build_metered_entitlement(meta: MeteredMeta, consumed: Decimal) -> EffectiveEntitlement {
    let remaining = meta.limit.map(|l| (l - consumed).max(Decimal::ZERO));
    // A grouped limit refuses per group: one group at its ceiling leaves the others their access.
    let limit_reached = meta.enabled
        && meta.group_by.is_none()
        && limit_reached(&meta.overage_behavior, meta.limit, consumed);
    let enabled = meta.enabled && !limit_reached;
    EffectiveEntitlement {
        feature: meta.feature,
        origin: meta.origin,
        own_grant: None,
        value: EffectiveEntitlementValue::Metered {
            metric_id: meta.metric_id,
            limit: meta.limit,
            reset_period: meta.reset_period,
            overage_behavior: meta.overage_behavior,
            thresholds: meta.thresholds,
            enabled,
            group_by: meta.group_by,
            usage: EntitlementUsage {
                consumed: Some(consumed),
                remaining,
                period_start: Some(meta.period_start.and_utc()),
                reset_at: meta.reset_at,
                unavailable: false,
            },
            limit_reached,
        },
    }
}

/// A metered entitlement without its usage: metering did not answer (`unavailable`), or the
/// usage cannot be counted (its metric is gone, or its level names no resource).
fn build_unread_metered_entitlement(meta: MeteredMeta, unavailable: bool) -> EffectiveEntitlement {
    EffectiveEntitlement {
        feature: meta.feature,
        origin: meta.origin,
        own_grant: None,
        value: EffectiveEntitlementValue::Metered {
            metric_id: meta.metric_id,
            limit: meta.limit,
            reset_period: meta.reset_period,
            overage_behavior: meta.overage_behavior,
            thresholds: meta.thresholds,
            enabled: meta.enabled,
            group_by: meta.group_by,
            usage: EntitlementUsage {
                consumed: None,
                remaining: None,
                period_start: None,
                reset_at: None,
                unavailable,
            },
            limit_reached: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    fn meta(limit: Option<f64>, period_start: NaiveDateTime) -> MeteredMeta {
        MeteredMeta {
            feature: FeatureRef {
                id: FeatureId::new(),
                name: "test".to_string(),
                code: "test".to_string(),
                product: None,
            },
            origin: ResolvedOrigin {
                entity: common_domain::ids::EntitlementEntityId::Feature(FeatureId::new()),
                name: None,
            },
            metric_id: BillableMetricId::new(),
            limit: limit.map(|v| Decimal::try_from(v).unwrap()),
            reset_period: ResetPeriod::Never,
            overage_behavior: OverageBehavior::Block {
                grace_period_pct: None,
            },
            thresholds: Vec::new(),
            enabled: true,
            period_start,
            reset_at: None,
            group_by: None,
        }
    }

    // --- build_metered_entitlement ---

    #[test]
    fn build_remaining_is_limit_minus_consumed() {
        let period_start = common_clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(40));
        let EffectiveEntitlementValue::Metered { usage, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert_eq!(usage.consumed, Some(Decimal::from(40)));
        assert_eq!(usage.remaining, Some(Decimal::from(60)));
    }

    #[test]
    fn build_remaining_clamped_at_zero_when_over_limit() {
        let period_start = common_clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(50.0), period_start), Decimal::from(80));
        let EffectiveEntitlementValue::Metered { usage, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert_eq!(usage.remaining, Some(Decimal::ZERO));
    }

    #[test]
    fn build_enabled_false_when_consumed_equals_limit() {
        let period_start = common_clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(100));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(!enabled);
    }

    #[test]
    fn build_enabled_false_when_consumed_over_limit() {
        let period_start = common_clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(150));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(!enabled);
    }

    #[test]
    fn build_enabled_true_when_under_limit() {
        let period_start = common_clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(99));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(enabled);
    }

    #[test]
    fn build_enabled_true_when_no_limit() {
        let period_start = common_clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(None, period_start), Decimal::from(999));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(enabled);
    }

    #[test]
    fn build_enabled_false_propagates_when_already_disabled() {
        let period_start = common_clock::now().naive_utc();
        let mut m = meta(Some(100.0), period_start);
        m.enabled = false;
        let ent = build_metered_entitlement(m, Decimal::from(10));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(!enabled);
    }

    fn enabled_and_reached(m: MeteredMeta, consumed: i64) -> (bool, bool) {
        let ent = build_metered_entitlement(m, Decimal::from(consumed));
        let EffectiveEntitlementValue::Metered {
            enabled,
            limit_reached,
            ..
        } = ent.value
        else {
            panic!("expected Metered");
        };
        (enabled, limit_reached)
    }

    #[test]
    fn an_allowing_limit_stays_enabled_past_the_limit() {
        let mut m = meta(Some(100.0), common_clock::now().naive_utc());
        m.overage_behavior = OverageBehavior::Allow;
        assert_eq!(enabled_and_reached(m, 130), (true, false));
        let mut zero = meta(Some(0.0), common_clock::now().naive_utc());
        zero.overage_behavior = OverageBehavior::Allow;
        assert_eq!(enabled_and_reached(zero, 5), (true, false));
    }

    #[test]
    fn a_blocking_limit_disables_only_once_its_grace_is_used() {
        let with_grace = || {
            let mut m = meta(Some(100.0), common_clock::now().naive_utc());
            m.overage_behavior = OverageBehavior::Block {
                grace_period_pct: Some(10),
            };
            m
        };
        assert_eq!(enabled_and_reached(with_grace(), 105), (true, false));
        assert_eq!(enabled_and_reached(with_grace(), 110), (false, true));
        assert_eq!(
            enabled_and_reached(meta(Some(100.0), common_clock::now().naive_utc()), 100),
            (false, true)
        );
    }

    #[test]
    fn a_switched_off_entitlement_is_not_reported_as_reached() {
        let mut m = meta(Some(100.0), common_clock::now().naive_utc());
        m.enabled = false;
        assert_eq!(enabled_and_reached(m, 500), (false, false));
    }

    #[test]
    fn one_group_at_its_ceiling_does_not_disable_a_grouped_entitlement() {
        let mut m = meta(Some(100.0), common_clock::now().naive_utc());
        m.group_by = Some("user_id".to_owned());
        assert_eq!(enabled_and_reached(m, 150), (true, false));
    }

    #[test]
    fn a_sliding_window_has_no_reset_moment() {
        let end = Some(common_clock::now().naive_utc());
        let sliding = ResetPeriod::SlidingWindow {
            unit: crate::domain::entitlements::PeriodUnit::Hour,
            interval: 5,
        };
        assert_eq!(resets_at(&sliding, end), None);
        assert_eq!(resets_at(&ResetPeriod::BillingCycle, end), end);
    }

    #[test]
    fn build_remaining_none_when_no_limit() {
        let period_start = common_clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(None, period_start), Decimal::from(999));
        let EffectiveEntitlementValue::Metered { usage, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert_eq!(usage.remaining, None);
    }

    #[test]
    fn build_period_start_matches_meta() {
        let period_start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let ent = build_metered_entitlement(meta(None, period_start), Decimal::ZERO);
        let EffectiveEntitlementValue::Metered { usage, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert_eq!(usage.period_start, Some(period_start.and_utc()));
    }

    #[test]
    fn unavailable_entitlement_preserves_spec_and_clears_usage() {
        let mut m = meta(Some(500.0), common_clock::now().naive_utc());
        m.reset_period = ResetPeriod::BillingCycle;
        m.overage_behavior = OverageBehavior::Block {
            grace_period_pct: Some(10),
        };
        m.thresholds = vec![80];
        let (feature, limit) = (m.feature.clone(), m.limit);
        let ent = build_unread_metered_entitlement(m, true);

        assert_eq!(ent.feature.id, feature.id);
        let EffectiveEntitlementValue::Metered {
            limit: l,
            thresholds,
            enabled,
            usage,
            ..
        } = ent.value
        else {
            panic!("expected Metered");
        };
        assert_eq!(l, limit);
        assert_eq!(thresholds, vec![80]);
        assert!(enabled);
        assert_eq!(usage.consumed, None);
        assert_eq!(usage.remaining, None);
        assert_eq!(usage.period_start, None);
        assert_eq!(usage.reset_at, None);
        assert!(usage.unavailable);
    }
}
