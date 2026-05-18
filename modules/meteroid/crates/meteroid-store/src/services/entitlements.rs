use crate::StoreResult;
use crate::domain::entitlements::{
    EffectiveEntitlement, EffectiveEntitlementValue, EntitlementUsage, FeatureRef, OverageBehavior,
    ResetPeriod, ResolvedEntitlement, ResolvedEntitlementValue, ResolvedOrigin,
};
use crate::domain::enums::BillingMetricAggregateEnum;
use crate::domain::{BillableMetric, UsagePeriod};
use crate::errors::StoreError;
use crate::repositories::EntitlementsInterface;
use crate::repositories::entitlements::{BillingCyclePeriod, compute_usage_period};
use crate::services::Services;
use crate::services::clients::usage::GroupedUsageData;
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::{BillableMetricId, CustomerId, FeatureId, TenantId};
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use futures::stream::{self, StreamExt, TryStreamExt};
use itertools::Itertools;
use rust_decimal::Decimal;
use std::collections::HashMap;

/// Cap on concurrent metering-backend fetches per `enrich_with_usage` call.
static USAGE_FETCH_CONCURRENCY: usize = 8;

struct MeteredMeta {
    feature: FeatureRef,
    created_at: DateTime<Utc>,
    origin: ResolvedOrigin,
    metric_id: BillableMetricId,
    aggregation_type: BillingMetricAggregateEnum,
    limit: Option<Decimal>,
    reset_period: ResetPeriod,
    overage_behavior: OverageBehavior,
    warning_threshold_pct: Option<u32>,
    enabled: bool,
    period_start: NaiveDateTime,
    reset_at: Option<DateTime<Utc>>,
}

impl Services {
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
                .get_effective_entitlements(&mut conn, customer_id, tenant_id)
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
                .get_effective_entitlements_for_feature(
                    &mut conn,
                    customer_id,
                    tenant_id,
                    feature_id,
                )
                .await?
        };
        let target: Vec<ResolvedEntitlement> = resolved.into_iter().collect::<Vec<_>>();
        let mut enriched = self
            .enrich_with_usage(customer_id, tenant_id, target)
            .await?;
        Ok(enriched.pop())
    }

    async fn enrich_with_usage(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
        resolved: Vec<ResolvedEntitlement>,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        if resolved.is_empty() {
            return Ok(vec![]);
        }

        let now = Utc::now();

        let metric_ids: Vec<BillableMetricId> = resolved
            .iter()
            .filter_map(|e| match &e.value {
                ResolvedEntitlementValue::Metered { metric_id, .. } => Some(*metric_id),
                ResolvedEntitlementValue::Boolean { .. } => None,
            })
            .unique()
            .collect();

        // Acquire a connection only for the local DB lookups (billing context + metric
        // metadata), then drop it before the HTTP fan-out so the pool isn't tied up while
        // waiting on the metering backend.
        let (billing_cycle_period, activation_date, metrics) = {
            let mut conn = self.store.get_conn().await?;
            let (bc, ad) = if metric_ids.is_empty() {
                (None, None)
            } else {
                self.load_billing_context(&mut conn, customer_id, tenant_id)
                    .await?
            };
            let metrics = self
                .load_metrics(&mut conn, &metric_ids, &tenant_id)
                .await?;
            (bc, ad, metrics)
        };

        let mut result: Vec<EffectiveEntitlement> = Vec::with_capacity(resolved.len());
        let mut metered_meta: Vec<MeteredMeta> = Vec::new();
        let mut usage_futs = Vec::new();

        for ent in resolved {
            match ent.value {
                ResolvedEntitlementValue::Boolean { enabled } => {
                    result.push(EffectiveEntitlement {
                        feature: ent.feature,
                        created_at: ent.created_at,
                        origin: ent.origin,
                        value: EffectiveEntitlementValue::Boolean { enabled },
                    });
                }
                ResolvedEntitlementValue::Metered {
                    metric_id,
                    limit,
                    reset_period,
                    overage_behavior,
                    warning_threshold_pct,
                    enabled,
                } => {
                    let Some(metric) = metrics.get(&metric_id) else {
                        // Metric row was deleted out from under an active entitlement.
                        log::warn!(
                            "metric {metric_id} not found for feature {} — emitting entitlement without usage",
                            ent.feature.id
                        );
                        result.push(build_unavailable_metered_entitlement(
                            ent.feature,
                            ent.created_at,
                            ent.origin,
                            metric_id,
                            limit,
                            reset_period,
                            overage_behavior,
                            warning_threshold_pct,
                            enabled,
                        ));
                        continue;
                    };

                    let bounds = compute_usage_period(
                        &reset_period,
                        billing_cycle_period,
                        activation_date,
                        now.naive_utc(),
                    );
                    let period_start = bounds.start;
                    let period_end = bounds.end.unwrap_or_else(|| now.naive_utc());
                    let reset_at = bounds.end.map(|t| t.and_utc());
                    let meta = MeteredMeta {
                        feature: ent.feature,
                        created_at: ent.created_at,
                        origin: ent.origin,
                        metric_id,
                        aggregation_type: metric.aggregation_type,
                        limit,
                        reset_period,
                        overage_behavior,
                        warning_threshold_pct,
                        enabled,
                        period_start,
                        reset_at,
                    };

                    // Empty window: metering rejects start>=end, so report zero usage directly
                    // instead of failing the whole call.
                    if period_start >= period_end {
                        result.push(build_metered_entitlement(meta, Decimal::ZERO));
                        continue;
                    }

                    usage_futs.push(self.usage_client.fetch_usage(
                        &tenant_id,
                        &customer_id,
                        metric,
                        UsagePeriod {
                            start: period_start,
                            end: period_end,
                        },
                    ));
                    metered_meta.push(meta);
                }
            }
        }

        let usage_results: Vec<_> = stream::iter(usage_futs)
            .buffered(USAGE_FETCH_CONCURRENCY)
            .try_collect()
            .await?;

        for (meta, usage_data) in metered_meta.into_iter().zip(usage_results) {
            let consumed = reduce_usage(&usage_data.data, meta.aggregation_type);
            result.push(build_metered_entitlement(meta, consumed));
        }

        Ok(result)
    }

    async fn load_billing_context(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<(Option<BillingCyclePeriod>, Option<NaiveDate>)> {
        let sub_rows = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // For customers with multiple active subscriptions, anchor to the most recently
        // started subscription's billing cycle. This is a deliberate heuristic: it picks
        // the most recently activated plan, which is typically the customer's "primary"
        // subscription. All metered entitlements share this single billing cycle anchor.
        let billing_cycle_period =
            sub_rows
                .iter()
                .max_by_key(|r| r.current_period_start)
                .map(|r| BillingCyclePeriod {
                    period_start: r.current_period_start,
                    period_end: r.current_period_end,
                });

        let activation_date = sub_rows
            .iter()
            .filter_map(|r| r.activated_at.map(|dt| dt.date()))
            .min();

        Ok((billing_cycle_period, activation_date))
    }

    async fn load_metrics(
        &self,
        conn: &mut PgConn,
        metric_ids: &[BillableMetricId],
        tenant_id: &TenantId,
    ) -> StoreResult<HashMap<BillableMetricId, BillableMetric>> {
        if metric_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let metric_rows = BillableMetricRow::get_by_ids(conn, metric_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        metric_rows
            .into_iter()
            .map(|row| -> Result<_, Report<StoreError>> {
                let m: BillableMetric = row.try_into()?;
                Ok((m.id, m))
            })
            .collect()
    }
}

/// Fold a metric's grouped usage rows into the single `consumed` number reported on the
/// effective entitlement.
///
/// Each [`GroupedUsageData`] is already pre-aggregated by the metering backend for one
/// dimension group (e.g. one region). This function combines those per-group values:
///
/// - `Sum` / `Count` / `CountDistinct` — sum the group values; the natural total across
///   dimensions.
/// - `Latest` — also summed across groups. `GroupedUsageData` has no per-group timestamp,
///   so we cannot pick the temporally-latest single group; summing the per-group latest
///   gives the current total (e.g. latest queue depth across regions).
/// - `Max` / `Min` — pick across group values.
/// - `Mean` — mean of per-group totals, **not** mean per event. For a metric with one
///   dimension group this equals `Sum`. Callers wanting per-event mean must aggregate
///   that way at the metering layer.
fn reduce_usage(
    data: &[GroupedUsageData],
    aggregation_type: BillingMetricAggregateEnum,
) -> Decimal {
    let values = || data.iter().map(|g| g.value);
    match aggregation_type {
        BillingMetricAggregateEnum::Sum
        | BillingMetricAggregateEnum::Count
        | BillingMetricAggregateEnum::CountDistinct
        | BillingMetricAggregateEnum::Latest => values().sum(),
        BillingMetricAggregateEnum::Max => values().reduce(Decimal::max).unwrap_or(Decimal::ZERO),
        BillingMetricAggregateEnum::Min => values().reduce(Decimal::min).unwrap_or(Decimal::ZERO),
        BillingMetricAggregateEnum::Mean => {
            if data.is_empty() {
                Decimal::ZERO
            } else {
                values().sum::<Decimal>() / Decimal::from(data.len())
            }
        }
    }
}

fn build_metered_entitlement(meta: MeteredMeta, consumed: Decimal) -> EffectiveEntitlement {
    let remaining = meta.limit.map(|l| (l - consumed).max(Decimal::ZERO));
    EffectiveEntitlement {
        feature: meta.feature,
        created_at: meta.created_at,
        origin: meta.origin,
        value: EffectiveEntitlementValue::Metered {
            metric_id: meta.metric_id,
            limit: meta.limit,
            reset_period: meta.reset_period,
            overage_behavior: meta.overage_behavior,
            warning_threshold_pct: meta.warning_threshold_pct,
            enabled: meta.enabled,
            usage: EntitlementUsage {
                consumed: Some(consumed),
                remaining,
                period_start: Some(meta.period_start.and_utc()),
                reset_at: meta.reset_at,
            },
        },
    }
}

/// Build a metered entitlement whose underlying metric row is missing — usage fields are all
/// `None` so the caller can render "usage unavailable" without losing the feature row.
#[allow(clippy::too_many_arguments)]
fn build_unavailable_metered_entitlement(
    feature: FeatureRef,
    created_at: DateTime<Utc>,
    origin: ResolvedOrigin,
    metric_id: BillableMetricId,
    limit: Option<Decimal>,
    reset_period: ResetPeriod,
    overage_behavior: OverageBehavior,
    warning_threshold_pct: Option<u32>,
    enabled: bool,
) -> EffectiveEntitlement {
    EffectiveEntitlement {
        feature,
        created_at,
        origin,
        value: EffectiveEntitlementValue::Metered {
            metric_id,
            limit,
            reset_period,
            overage_behavior,
            warning_threshold_pct,
            enabled,
            usage: EntitlementUsage {
                consumed: None,
                remaining: None,
                period_start: None,
                reset_at: None,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    fn group(value: f64) -> GroupedUsageData {
        GroupedUsageData {
            value: Decimal::try_from(value).unwrap(),
            dimensions: std::collections::HashMap::new(),
        }
    }

    fn meta(limit: Option<f64>, period_start: NaiveDateTime) -> MeteredMeta {
        MeteredMeta {
            feature: FeatureRef {
                id: FeatureId::new(),
                name: "test".to_string(),
                product: None,
            },
            created_at: Utc::now(),
            origin: ResolvedOrigin {
                entity: common_domain::ids::EntitlementEntityId::Feature(FeatureId::new()),
                name: None,
            },
            metric_id: BillableMetricId::new(),
            aggregation_type: BillingMetricAggregateEnum::Sum,
            limit: limit.map(|v| Decimal::try_from(v).unwrap()),
            reset_period: ResetPeriod::Never,
            overage_behavior: OverageBehavior::Block {
                grace_period_pct: None,
            },
            warning_threshold_pct: None,
            enabled: true,
            period_start,
            reset_at: None,
        }
    }

    // --- reduce_usage ---

    #[test]
    fn reduce_sum_adds_groups() {
        let data = vec![group(10.0), group(20.0), group(30.0)];
        assert_eq!(
            reduce_usage(&data, BillingMetricAggregateEnum::Sum),
            Decimal::from(60)
        );
    }

    #[test]
    fn reduce_count_adds_groups() {
        let data = vec![group(5.0), group(3.0)];
        assert_eq!(
            reduce_usage(&data, BillingMetricAggregateEnum::Count),
            Decimal::from(8)
        );
    }

    #[test]
    fn reduce_count_distinct_adds_groups() {
        let data = vec![group(100.0), group(50.0)];
        assert_eq!(
            reduce_usage(&data, BillingMetricAggregateEnum::CountDistinct),
            Decimal::from(150)
        );
    }

    #[test]
    fn reduce_max_takes_largest() {
        let data = vec![group(10.0), group(30.0), group(20.0)];
        assert_eq!(
            reduce_usage(&data, BillingMetricAggregateEnum::Max),
            Decimal::from(30)
        );
    }

    #[test]
    fn reduce_latest_sums_groups() {
        // Each GroupedUsageData already holds the per-group latest value. Across groups
        // we sum (e.g. latest queue depth per region → total queue depth).
        let data = vec![group(5.0), group(15.0)];
        assert_eq!(
            reduce_usage(&data, BillingMetricAggregateEnum::Latest),
            Decimal::from(20)
        );
    }

    #[test]
    fn reduce_max_preserves_negative_values() {
        let data = vec![group(-10.0), group(-5.0), group(-20.0)];
        assert_eq!(
            reduce_usage(&data, BillingMetricAggregateEnum::Max),
            Decimal::from(-5)
        );
    }

    #[test]
    fn reduce_min_preserves_positive_values_only() {
        let data = vec![group(10.0), group(20.0)];
        assert_eq!(
            reduce_usage(&data, BillingMetricAggregateEnum::Min),
            Decimal::from(10)
        );
    }

    #[test]
    fn reduce_min_takes_smallest() {
        let data = vec![group(10.0), group(5.0), group(20.0)];
        assert_eq!(
            reduce_usage(&data, BillingMetricAggregateEnum::Min),
            Decimal::from(5)
        );
    }

    #[test]
    fn reduce_mean_averages_groups() {
        let data = vec![group(10.0), group(20.0), group(30.0)];
        assert_eq!(
            reduce_usage(&data, BillingMetricAggregateEnum::Mean),
            Decimal::from(20)
        );
    }

    #[test]
    fn reduce_empty_returns_zero_for_all_types() {
        for agg in [
            BillingMetricAggregateEnum::Sum,
            BillingMetricAggregateEnum::Count,
            BillingMetricAggregateEnum::CountDistinct,
            BillingMetricAggregateEnum::Max,
            BillingMetricAggregateEnum::Latest,
            BillingMetricAggregateEnum::Min,
            BillingMetricAggregateEnum::Mean,
        ] {
            assert_eq!(reduce_usage(&[], agg), Decimal::ZERO, "failed for {agg:?}");
        }
    }

    // --- build_metered_entitlement ---

    #[test]
    fn build_remaining_is_limit_minus_consumed() {
        let period_start = Utc::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(40));
        let EffectiveEntitlementValue::Metered { usage, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert_eq!(usage.consumed, Some(Decimal::from(40)));
        assert_eq!(usage.remaining, Some(Decimal::from(60)));
    }

    #[test]
    fn build_remaining_clamped_at_zero_when_over_limit() {
        let period_start = Utc::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(50.0), period_start), Decimal::from(80));
        let EffectiveEntitlementValue::Metered { usage, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert_eq!(usage.remaining, Some(Decimal::ZERO));
    }

    #[test]
    fn build_remaining_none_when_no_limit() {
        let period_start = Utc::now().naive_utc();
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
            product: None,
        };
        let origin = ResolvedOrigin {
            entity: common_domain::ids::EntitlementEntityId::Feature(feature.id),
            name: None,
        };
        let created_at = Utc::now();
        let metric_id = BillableMetricId::new();
        let limit = Some(Decimal::from(500));

        let ent = build_unavailable_metered_entitlement(
            feature.clone(),
            created_at,
            origin,
            metric_id,
            limit,
            ResetPeriod::BillingCycle,
            OverageBehavior::Block {
                grace_period_pct: Some(10),
            },
            Some(80),
            true,
        );

        assert_eq!(ent.feature.id, feature.id);
        assert_eq!(ent.created_at, created_at);
        let EffectiveEntitlementValue::Metered {
            limit: l,
            warning_threshold_pct,
            enabled,
            usage,
            ..
        } = ent.value
        else {
            panic!("expected Metered");
        };
        assert_eq!(l, limit);
        assert_eq!(warning_threshold_pct, Some(80));
        assert!(enabled);
        assert_eq!(usage.consumed, None);
        assert_eq!(usage.remaining, None);
        assert_eq!(usage.period_start, None);
        assert_eq!(usage.reset_at, None);
    }
}
