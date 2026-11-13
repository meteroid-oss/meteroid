pub(crate) mod limits;

use crate::StoreResult;
use crate::domain::entitlements::{
    EffectiveEntitlement, EffectiveEntitlementValue, EntitlementUsage, FeatureRef, OverageBehavior,
    ResetPeriod, ResolvedEntitlement, ResolvedEntitlementValue, ResolvedOrigin,
};
use crate::domain::{BillableMetric, UsagePeriod};
use crate::errors::StoreError;
use crate::hooks::ExpectedMonitor;
use crate::repositories::EntitlementsInterface;
use crate::repositories::entitlements::{BillingCyclePeriod, ResolveTarget, compute_usage_period};
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::repositories::subscriptions::usage_attribution::usage_scopes_of;
use crate::services::Services;
use crate::services::entitlements::limits::rules::hard_limit_reached;
use crate::services::entitlements::limits::{
    anchor_subscription, earliest_activation, entitlement_monitor_id, largest_group, load_metrics,
};
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::ids::{BillableMetricId, CustomerId, FeatureId, SubscriptionId, TenantId};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use futures::stream::{self, StreamExt, TryStreamExt};
use itertools::Itertools;
use rust_decimal::Decimal;
use std::collections::HashMap;
use uuid::Uuid;

/// Cap on concurrent metering-backend fetches per `enrich_with_usage` call.
pub const USAGE_FETCH_CONCURRENCY: usize = 8;

/// The cycle and activation the usage windows of a customer's entitlements are anchored to, and
/// the subscription whose usage filter measures them.
#[derive(Clone, Copy)]
pub(crate) struct BillingContext {
    cycle: Option<BillingCyclePeriod>,
    activation: Option<NaiveDateTime>,
    pub(crate) subscription_id: Option<SubscriptionId>,
}

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

struct PendingUsage<'a> {
    meta: MeteredMeta,
    metric: &'a BillableMetric,
    period: UsagePeriod,
}

/// A subscription's current cycle, as `limited_usage_for_subscription` reads it.
#[derive(Clone, Copy, Debug)]
pub struct SubscriptionCycle {
    pub subscription_id: SubscriptionId,
    pub customer_id: CustomerId,
    pub period_start: NaiveDate,
    /// Exclusive, as `subscription.current_period_end`.
    pub period_end: Option<NaiveDate>,
    pub activated_at: Option<NaiveDateTime>,
}

/// An enabled metered entitlement with a positive limit and its usage over its window.
#[derive(Clone, Debug)]
pub struct LimitedUsage {
    pub feature: FeatureRef,
    pub metric_id: BillableMetricId,
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

impl Services {
    /// Usage against every limited entitlement of the subscription at `now`, with billing-cycle
    /// limits bounded by the subscription's own cycle. Empty once the cycle has ended and the
    /// renewal is not processed yet: that usage belongs to a period no longer current.
    pub(crate) async fn limited_usage_for_subscription(
        &self,
        tenant_id: TenantId,
        sub: &SubscriptionCycle,
        now: NaiveDateTime,
        concurrency: usize,
    ) -> StoreResult<Vec<LimitedUsage>> {
        if sub
            .period_end
            .is_some_and(|end| now >= end.and_time(NaiveTime::MIN))
        {
            return Ok(vec![]);
        }
        let (resolved, metrics) = {
            let mut conn = self.store.get_conn().await?;
            let resolved = self
                .store
                .resolve_entitlements_for_entity(
                    &mut conn,
                    tenant_id,
                    ResolveTarget::Subscription(sub.subscription_id),
                )
                .await?;
            let metric_ids: Vec<BillableMetricId> = resolved
                .iter()
                .filter_map(|e| match &e.value {
                    ResolvedEntitlementValue::Metered { metric_id, .. } => Some(*metric_id),
                    _ => None,
                })
                .unique()
                .collect();
            let metrics = self
                .load_metrics(
                    &mut conn,
                    &metric_ids,
                    &tenant_id,
                    Some(sub.subscription_id),
                )
                .await?;
            (resolved, metrics)
        };

        let billing_cycle = Some(BillingCyclePeriod {
            period_start: sub.period_start,
            period_end: sub.period_end,
        });
        let limited: Vec<(LimitedUsage, &BillableMetric, Option<String>)> = resolved
            .into_iter()
            .filter_map(|ent| {
                let ResolvedEntitlementValue::Metered {
                    metric_id,
                    limit: Some(limit),
                    reset_period,
                    overage_behavior,
                    thresholds,
                    enabled: true,
                    group_by,
                } = ent.value
                else {
                    return None;
                };
                if limit <= Decimal::ZERO {
                    return None;
                }
                let Some(metric) = metrics.get(&metric_id) else {
                    log::warn!(
                        "metric {metric_id} not found for feature {}",
                        ent.feature.id
                    );
                    return None;
                };
                let bounds =
                    compute_usage_period(&reset_period, billing_cycle, sub.activated_at, now);
                let period_end = resets_at(&reset_period, bounds.end);
                let usage = LimitedUsage {
                    feature: ent.feature,
                    metric_id,
                    limit,
                    consumed: Decimal::ZERO,
                    reset_period,
                    overage_behavior,
                    thresholds,
                    period_start: bounds.start,
                    period_end,
                    group_by: group_by.clone(),
                };
                Some((usage, metric, group_by))
            })
            .collect();

        let expected = limited
            .iter()
            .filter_map(|(usage, _, group_by)| {
                engine_monitor(
                    sub.subscription_id,
                    usage.feature.id,
                    &usage.reset_period,
                    usage.period_start,
                    usage.limit,
                    group_by.is_some(),
                )
            })
            .collect::<Vec<_>>();
        let engine = self
            .engine_usage(tenant_id, sub.customer_id, &expected, now)
            .await;

        // Futures collected first: a stream `.map` closure borrowing the metrics makes the
        // caller's future fail the higher-ranked `Send` check when spawned.
        let fetches: Vec<_> = limited
            .into_iter()
            .map(|(usage, metric, group_by)| {
                let engine_consumed = engine
                    .get(&entitlement_monitor_id(
                        usage.feature.id,
                        sub.subscription_id,
                    ))
                    .copied();
                async move {
                    let window_end = usage.period_end.map_or(now, |end| end.min(now));
                    if usage.period_start >= window_end {
                        return Ok(usage);
                    }
                    if let Some(consumed) = engine_consumed {
                        return Ok(LimitedUsage { consumed, ..usage });
                    }
                    let consumed = self
                        .metering_usage(
                            &tenant_id,
                            &sub.customer_id,
                            metric,
                            group_by.as_deref(),
                            UsagePeriod {
                                start: usage.period_start,
                                end: window_end,
                            },
                        )
                        .await?;
                    Ok(LimitedUsage { consumed, ..usage })
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
        // Scoped conn so it returns to the pool before `enrich_with_usage` fans out
        // potentially many metering HTTP requests.
        let resolved = {
            let mut conn = self.store.get_conn().await?;
            self.store
                .resolve_entitlements_for_customer(&mut conn, customer_id, tenant_id)
                .await?
        };
        self.enrich_with_usage(customer_id, tenant_id, resolved)
            .await
    }

    pub(crate) async fn get_effective_entitlement_for_feature(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
        feature_id: FeatureId,
    ) -> StoreResult<Option<EffectiveEntitlement>> {
        let resolved = {
            let mut conn = self.store.get_conn().await?;
            self.store
                .resolve_entitlements_for_feature(&mut conn, customer_id, tenant_id, feature_id)
                .await?
        };
        let target: Vec<ResolvedEntitlement> = resolved.into_iter().collect::<Vec<_>>();
        let mut enriched = self
            .enrich_with_usage(customer_id, tenant_id, target)
            .await?;
        Ok(enriched.pop())
    }

    pub(crate) async fn get_effective_entitlements_for_subscription(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        let sub = self
            .store
            .get_subscription(tenant_id, subscription_id)
            .await?;
        let resolved = {
            let mut conn = self.store.get_conn().await?;
            self.store
                .resolve_entitlements_for_entity(
                    &mut conn,
                    tenant_id,
                    ResolveTarget::Subscription(subscription_id),
                )
                .await?
        };
        let context = BillingContext {
            cycle: Some(BillingCyclePeriod {
                period_start: sub.current_period_start,
                period_end: sub.current_period_end,
            }),
            activation: sub.activated_at,
            subscription_id: Some(subscription_id),
        };
        self.enrich_with_context(sub.customer_id, tenant_id, resolved, context)
            .await
    }

    async fn enrich_with_usage(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
        resolved: Vec<ResolvedEntitlement>,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        let metric_ids: Vec<BillableMetricId> = resolved
            .iter()
            .filter_map(|e| match &e.value {
                ResolvedEntitlementValue::Metered { metric_id, .. } => Some(*metric_id),
                ResolvedEntitlementValue::Boolean { .. }
                | ResolvedEntitlementValue::Config { .. } => None,
            })
            .unique()
            .collect();

        let context = if metric_ids.is_empty() {
            BillingContext {
                cycle: None,
                activation: None,
                subscription_id: None,
            }
        } else {
            let mut conn = self.store.get_conn().await?;
            self.load_billing_context(&mut conn, customer_id, tenant_id)
                .await?
        };

        self.enrich_with_context(customer_id, tenant_id, resolved, context)
            .await
    }

    async fn enrich_with_context(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
        resolved: Vec<ResolvedEntitlement>,
        context: BillingContext,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        if resolved.is_empty() {
            return Ok(vec![]);
        }

        let now = common_domain::clock::now();

        let metric_ids: Vec<BillableMetricId> = resolved
            .iter()
            .filter_map(|e| match &e.value {
                ResolvedEntitlementValue::Metered { metric_id, .. } => Some(*metric_id),
                ResolvedEntitlementValue::Boolean { .. }
                | ResolvedEntitlementValue::Config { .. } => None,
            })
            .unique()
            .collect();

        let metrics = {
            let mut conn = self.store.get_conn().await?;
            self.load_metrics(&mut conn, &metric_ids, &tenant_id, context.subscription_id)
                .await?
        };

        let mut result: Vec<EffectiveEntitlement> = Vec::with_capacity(resolved.len());
        let mut pending: Vec<PendingUsage<'_>> = Vec::new();

        for ent in resolved {
            match ent.value {
                ResolvedEntitlementValue::Boolean { enabled } => {
                    result.push(EffectiveEntitlement {
                        feature: ent.feature,
                        origin: ent.origin,
                        value: EffectiveEntitlementValue::Boolean { enabled },
                    });
                }
                ResolvedEntitlementValue::Config { value } => {
                    result.push(EffectiveEntitlement {
                        feature: ent.feature,
                        origin: ent.origin,
                        value: EffectiveEntitlementValue::Config { value },
                    });
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
                    let Some(metric) = metrics.get(&metric_id) else {
                        // Metric row was deleted out from under an active entitlement.
                        log::warn!(
                            "metric {metric_id} not found for feature {} — emitting entitlement without usage",
                            ent.feature.id
                        );
                        result.push(build_unavailable_metered_entitlement(
                            ent.feature,
                            ent.origin,
                            metric_id,
                            limit,
                            reset_period,
                            overage_behavior,
                            thresholds,
                            enabled,
                            group_by,
                        ));
                        continue;
                    };

                    let bounds = compute_usage_period(
                        &reset_period,
                        context.cycle,
                        context.activation,
                        now.naive_utc(),
                    );
                    let period_start = bounds.start;
                    let period_end = bounds.end.unwrap_or_else(|| now.naive_utc());
                    let reset_at = resets_at(&reset_period, bounds.end).map(|t| t.and_utc());
                    let meta = MeteredMeta {
                        feature: ent.feature,
                        origin: ent.origin,
                        metric_id,
                        limit,
                        reset_period,
                        overage_behavior,
                        thresholds,
                        enabled,
                        period_start,
                        reset_at,
                        group_by,
                    };

                    // Empty window: metering rejects start>=end, so report zero usage directly
                    // instead of failing the whole call.
                    if period_start >= period_end {
                        result.push(build_metered_entitlement(meta, Decimal::ZERO));
                        continue;
                    }

                    pending.push(PendingUsage {
                        meta,
                        metric,
                        period: UsagePeriod {
                            start: period_start,
                            end: period_end,
                        },
                    });
                }
            }
        }

        // Read live on every call, never from the engine's state: callers of this public read
        // expect usage ingested a moment ago, which the state only holds after its next sweep.
        let usage_futs: Vec<_> = pending
            .iter()
            .map(|p| {
                self.metering_usage(
                    &tenant_id,
                    &customer_id,
                    p.metric,
                    p.meta.group_by.as_deref(),
                    p.period.clone(),
                )
            })
            .collect();
        let consumed = stream::iter(usage_futs)
            .buffered(USAGE_FETCH_CONCURRENCY)
            .try_collect::<Vec<Decimal>>()
            .await?;

        for (p, consumed) in pending.into_iter().zip(consumed) {
            result.push(build_metered_entitlement(p.meta, consumed));
        }

        Ok(result)
    }

    /// What the entitlement's limit is measured against over the window: the customer's usage, or
    /// its largest group's when the limit applies per group, added up as the engine does.
    async fn metering_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: Option<&str>,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        match group_by {
            None => {
                self.usage_client
                    .fetch_total_usage(tenant_id, customer_id, metric, period)
                    .await
            }
            Some(property) => {
                let usage = self
                    .usage_client
                    .fetch_usage_grouped_by(tenant_id, customer_id, metric, property, period)
                    .await?;
                largest_group(&usage.data, property)
            }
        }
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

    pub(crate) async fn load_billing_context(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<BillingContext> {
        let sub_rows = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // The customer's windows follow one subscription: the same one the usage check follows.
        let anchor = anchor_subscription(&sub_rows);

        Ok(BillingContext {
            cycle: anchor.map(|r| BillingCyclePeriod {
                period_start: r.current_period_start,
                period_end: r.current_period_end,
            }),
            activation: earliest_activation(&sub_rows),
            subscription_id: anchor.map(|r| r.id),
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

/// The monitor the engine keeps for a limited entitlement window; none for a sliding window,
/// which it does not monitor.
fn engine_monitor(
    subscription_id: SubscriptionId,
    feature_id: FeatureId,
    reset_period: &ResetPeriod,
    period_start: NaiveDateTime,
    limit: Decimal,
    grouped: bool,
) -> Option<ExpectedMonitor> {
    if matches!(reset_period, ResetPeriod::SlidingWindow { .. }) {
        return None;
    }
    Some(ExpectedMonitor {
        id: entitlement_monitor_id(feature_id, subscription_id),
        window_start: period_start.and_utc(),
        limit,
        grouped,
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
            },
            limit_reached,
        },
    }
}

/// Build a metered entitlement whose underlying metric row is missing — usage fields are all
/// `None` so the caller can render "usage unavailable" without losing the feature row.
fn build_unavailable_metered_entitlement(
    feature: FeatureRef,
    origin: ResolvedOrigin,
    metric_id: BillableMetricId,
    limit: Option<Decimal>,
    reset_period: ResetPeriod,
    overage_behavior: OverageBehavior,
    thresholds: Vec<u32>,
    enabled: bool,
    group_by: Option<String>,
) -> EffectiveEntitlement {
    EffectiveEntitlement {
        feature,
        origin,
        value: EffectiveEntitlementValue::Metered {
            metric_id,
            limit,
            reset_period,
            overage_behavior,
            thresholds,
            enabled,
            group_by,
            usage: EntitlementUsage {
                consumed: None,
                remaining: None,
                period_start: None,
                reset_at: None,
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
        let period_start = common_domain::clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(40));
        let EffectiveEntitlementValue::Metered { usage, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert_eq!(usage.consumed, Some(Decimal::from(40)));
        assert_eq!(usage.remaining, Some(Decimal::from(60)));
    }

    #[test]
    fn build_remaining_clamped_at_zero_when_over_limit() {
        let period_start = common_domain::clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(50.0), period_start), Decimal::from(80));
        let EffectiveEntitlementValue::Metered { usage, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert_eq!(usage.remaining, Some(Decimal::ZERO));
    }

    #[test]
    fn build_enabled_false_when_consumed_equals_limit() {
        let period_start = common_domain::clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(100));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(!enabled);
    }

    #[test]
    fn build_enabled_false_when_consumed_over_limit() {
        let period_start = common_domain::clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(150));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(!enabled);
    }

    #[test]
    fn build_enabled_true_when_under_limit() {
        let period_start = common_domain::clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(99));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(enabled);
    }

    #[test]
    fn build_enabled_true_when_no_limit() {
        let period_start = common_domain::clock::now().naive_utc();
        let ent = build_metered_entitlement(meta(None, period_start), Decimal::from(999));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(enabled);
    }

    #[test]
    fn build_enabled_false_propagates_when_already_disabled() {
        let period_start = common_domain::clock::now().naive_utc();
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
        let mut m = meta(Some(100.0), common_domain::clock::now().naive_utc());
        m.overage_behavior = OverageBehavior::Allow;
        assert_eq!(enabled_and_reached(m, 130), (true, false));
        let mut zero = meta(Some(0.0), common_domain::clock::now().naive_utc());
        zero.overage_behavior = OverageBehavior::Allow;
        assert_eq!(enabled_and_reached(zero, 5), (true, false));
    }

    #[test]
    fn a_blocking_limit_disables_only_once_its_grace_is_used() {
        let with_grace = || {
            let mut m = meta(Some(100.0), common_domain::clock::now().naive_utc());
            m.overage_behavior = OverageBehavior::Block {
                grace_period_pct: Some(10),
            };
            m
        };
        assert_eq!(enabled_and_reached(with_grace(), 105), (true, false));
        assert_eq!(enabled_and_reached(with_grace(), 110), (false, true));
        assert_eq!(
            enabled_and_reached(
                meta(Some(100.0), common_domain::clock::now().naive_utc()),
                100
            ),
            (false, true)
        );
    }

    #[test]
    fn a_switched_off_entitlement_is_not_reported_as_reached() {
        let mut m = meta(Some(100.0), common_domain::clock::now().naive_utc());
        m.enabled = false;
        assert_eq!(enabled_and_reached(m, 500), (false, false));
    }

    #[test]
    fn one_group_at_its_ceiling_does_not_disable_a_grouped_entitlement() {
        let mut m = meta(Some(100.0), common_domain::clock::now().naive_utc());
        m.group_by = Some("user_id".to_owned());
        assert_eq!(enabled_and_reached(m, 150), (true, false));
    }

    #[test]
    fn a_sliding_window_has_no_reset_moment() {
        let end = Some(common_domain::clock::now().naive_utc());
        let sliding = ResetPeriod::SlidingWindow {
            unit: crate::domain::entitlements::PeriodUnit::Hour,
            interval: 5,
        };
        assert_eq!(resets_at(&sliding, end), None);
        assert_eq!(resets_at(&ResetPeriod::BillingCycle, end), end);
    }

    #[test]
    fn build_remaining_none_when_no_limit() {
        let period_start = common_domain::clock::now().naive_utc();
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
        // The "metric row deleted" branch in enrich_with_usage must keep the entitlement's
        // spec (limit/reset/overage/enabled) intact so the caller can still render the
        // feature, while signalling unavailable usage by zeroing every usage field.
        let feature = FeatureRef {
            id: FeatureId::new(),
            name: "deleted-metric".to_string(),
            code: "deleted-metric".to_string(),
            product: None,
        };
        let origin = ResolvedOrigin {
            entity: common_domain::ids::EntitlementEntityId::Feature(feature.id),
            name: None,
        };
        let metric_id = BillableMetricId::new();
        let limit = Some(Decimal::from(500));

        let ent = build_unavailable_metered_entitlement(
            feature.clone(),
            origin,
            metric_id,
            limit,
            ResetPeriod::BillingCycle,
            OverageBehavior::Block {
                grace_period_pct: Some(10),
            },
            vec![80],
            true,
            None,
        );

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
    }
}
