use crate::StoreResult;
use crate::domain::entitlements::{
    EffectiveEntitlement, EffectiveEntitlementValue, EntitlementUsage, FeatureRef, OverageBehavior,
    ResetPeriod, ResolvedEntitlement, ResolvedEntitlementValue, ResolvedOrigin,
};
use crate::domain::{BillableMetric, UsagePeriod};
use crate::errors::StoreError;
use crate::repositories::EntitlementsInterface;
use crate::repositories::entitlements::{BillingCyclePeriod, ResolveTarget, compute_usage_period};
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::Services;

use crate::store::PgConn;
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{BillableMetricId, CustomerId, FeatureId, SubscriptionId, TenantId};
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
    origin: ResolvedOrigin,
    metric_id: BillableMetricId,
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
        let billing_cycle_period = Some(BillingCyclePeriod {
            period_start: sub.current_period_start,
            period_end: sub.current_period_end,
        });
        self.enrich_with_context(
            sub.customer_id,
            tenant_id,
            resolved,
            billing_cycle_period,
            sub.activated_at,
        )
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
                ResolvedEntitlementValue::Boolean { .. } => None,
            })
            .unique()
            .collect();

        let (billing_cycle_period, activation_date) = if metric_ids.is_empty() {
            (None, None)
        } else {
            let mut conn = self.store.get_conn().await?;
            self.load_billing_context(&mut conn, customer_id, tenant_id)
                .await?
        };

        self.enrich_with_context(
            customer_id,
            tenant_id,
            resolved,
            billing_cycle_period,
            activation_date,
        )
        .await
    }

    async fn enrich_with_context(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
        resolved: Vec<ResolvedEntitlement>,
        billing_cycle_period: Option<BillingCyclePeriod>,
        activation_date: Option<NaiveDateTime>,
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

        let metrics = {
            let mut conn = self.store.get_conn().await?;
            self.load_metrics(&mut conn, &metric_ids, &tenant_id)
                .await?
        };

        let mut result: Vec<EffectiveEntitlement> = Vec::with_capacity(resolved.len());
        let mut metered_meta: Vec<MeteredMeta> = Vec::new();
        let mut usage_futs = Vec::new();

        for ent in resolved {
            match ent.value {
                ResolvedEntitlementValue::Boolean { enabled } => {
                    result.push(EffectiveEntitlement {
                        feature: ent.feature,
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
                        origin: ent.origin,
                        metric_id,
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

                    usage_futs.push(self.usage_client.fetch_total_usage(
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

        for (meta, consumed) in metered_meta.into_iter().zip(usage_results) {
            result.push(build_metered_entitlement(meta, consumed));
        }

        Ok(result)
    }

    async fn load_billing_context(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<(Option<BillingCyclePeriod>, Option<NaiveDateTime>)> {
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

        let activation_date = sub_rows.iter().filter_map(|r| r.activated_at).min();

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

fn build_metered_entitlement(meta: MeteredMeta, consumed: Decimal) -> EffectiveEntitlement {
    let remaining = meta.limit.map(|l| (l - consumed).max(Decimal::ZERO));
    // Auto-disable when usage has reached or exceeded the limit.
    let enabled = meta.enabled && meta.limit.is_none_or(|l| consumed < l);
    EffectiveEntitlement {
        feature: meta.feature,
        origin: meta.origin,
        value: EffectiveEntitlementValue::Metered {
            metric_id: meta.metric_id,
            limit: meta.limit,
            reset_period: meta.reset_period,
            overage_behavior: meta.overage_behavior,
            warning_threshold_pct: meta.warning_threshold_pct,
            enabled,
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

    fn meta(limit: Option<f64>, period_start: NaiveDateTime) -> MeteredMeta {
        MeteredMeta {
            feature: FeatureRef {
                id: FeatureId::new(),
                name: "test".to_string(),
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
            warning_threshold_pct: None,
            enabled: true,
            period_start,
            reset_at: None,
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
    fn build_enabled_false_when_consumed_equals_limit() {
        let period_start = Utc::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(100));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(!enabled);
    }

    #[test]
    fn build_enabled_false_when_consumed_over_limit() {
        let period_start = Utc::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(150));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(!enabled);
    }

    #[test]
    fn build_enabled_true_when_under_limit() {
        let period_start = Utc::now().naive_utc();
        let ent = build_metered_entitlement(meta(Some(100.0), period_start), Decimal::from(99));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(enabled);
    }

    #[test]
    fn build_enabled_true_when_no_limit() {
        let period_start = Utc::now().naive_utc();
        let ent = build_metered_entitlement(meta(None, period_start), Decimal::from(999));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(enabled);
    }

    #[test]
    fn build_enabled_false_propagates_when_already_disabled() {
        let period_start = Utc::now().naive_utc();
        let mut m = meta(Some(100.0), period_start);
        m.enabled = false;
        let ent = build_metered_entitlement(m, Decimal::from(10));
        let EffectiveEntitlementValue::Metered { enabled, .. } = ent.value else {
            panic!("expected Metered");
        };
        assert!(!enabled);
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
            Some(80),
            true,
        );

        assert_eq!(ent.feature.id, feature.id);
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
