use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::BillingMetricAggregateEnum;
use crate::domain::{
    AttributedSubscription, AttributionPreview, BillableMetric, BillableMetricUpdate,
    MetricAttribution, UsagePeriod, scopes_for, unclaimed_scope,
};
use crate::repositories::billable_metrics::{
    BillableMetricInterface, attribution_drafts, validate_attribution,
};
use crate::repositories::subscriptions::usage_attribution::{
    attributed_subscriptions, customers_with_resource_subscriptions, priced_for_a_resource,
};
use crate::services::ServicesEdge;
use common_domain::ids::{BillableMetricId, CustomerId, TenantId};
use rust_decimal::Decimal;

/// Customers a preview reads: the split only differs for those with subscriptions for a resource.
const PREVIEW_SAMPLE: usize = 20;
const PREVIEW_DAYS: i64 = 7;

/// Whether quantities read over parts of a metric's events add up to the whole.
fn additive(aggregation: &BillingMetricAggregateEnum) -> bool {
    matches!(
        aggregation,
        BillingMetricAggregateEnum::Sum | BillingMetricAggregateEnum::Count
    )
}

/// The parts of `period` over which `subscription` prices `metric_id`, its rows' spans merged
/// where they overlap or touch: each is read once, as one line would bill it.
fn stretches(
    subscription: &AttributedSubscription,
    metric_id: BillableMetricId,
    period: &UsagePeriod,
) -> Vec<UsagePeriod> {
    let mut parts = subscription
        .spans
        .iter()
        .filter(|span| span.metric_id == metric_id)
        .map(|span| UsagePeriod {
            start: span.from.max(period.start),
            end: span.until.map_or(period.end, |until| until.min(period.end)),
        })
        .filter(|part| part.start < part.end)
        .collect::<Vec<_>>();
    parts.sort_by_key(|part| part.start);
    let mut merged: Vec<UsagePeriod> = Vec::with_capacity(parts.len());
    for part in parts {
        match merged.last_mut() {
            Some(last) if part.start <= last.end => last.end = last.end.max(part.end),
            _ => merged.push(part),
        }
    }
    merged
}

impl ServicesEdge {
    /// The store update: a changed attribution marks the drafts it moves usage of for the draft
    /// worker; finalized invoices never change (§3.6).
    pub async fn update_billable_metric(
        &self,
        actor: Actor,
        id: BillableMetricId,
        tenant_id: TenantId,
        update: BillableMetricUpdate,
    ) -> StoreResult<BillableMetric> {
        self.store
            .update_billable_metric(actor, id, tenant_id, update)
            .await
    }

    /// Where the last 7 days of `metric_id`'s usage would go under `attributed_to`, sampled over
    /// customers with subscriptions for a resource; also the drafts the change would recompute.
    pub async fn preview_metric_attribution(
        &self,
        tenant_id: TenantId,
        metric_id: BillableMetricId,
        attributed_to: Option<MetricAttribution>,
    ) -> StoreResult<AttributionPreview> {
        let mut metric = self
            .store
            .find_billable_metric_by_id(metric_id, tenant_id)
            .await?;
        let mut conn = self.store.get_conn().await?;
        if let Some(attribution) = &attributed_to {
            validate_attribution(&mut conn, tenant_id, attribution).await?;
        }
        (metric.attributed_to_type_id, metric.attribution_property) =
            attributed_to.map_or((None, None), |a| (Some(a.type_id), a.property));
        let until = common_clock::now().naive_utc();
        let period = UsagePeriod {
            start: until - chrono::Duration::days(PREVIEW_DAYS),
            end: until,
        };
        let customers = customers_with_resource_subscriptions(&mut conn, tenant_id).await?;
        let pricing = priced_for_a_resource(&mut conn, tenant_id, metric_id).await?;
        let mut preview = AttributionPreview {
            from: period.start,
            until: period.end,
            sampled_customers: customers.len().min(PREVIEW_SAMPLE) as u32,
            sampled: customers.len() > PREVIEW_SAMPLE,
            additive: additive(&metric.aggregation_type),
            resource_subscriptions: Decimal::ZERO,
            customer_subscriptions: Decimal::ZERO,
            not_billed: Decimal::ZERO,
            draft_invoices: attribution_drafts(&mut conn, tenant_id, &pricing).await? as u32,
        };
        for customer_id in customers.into_iter().take(PREVIEW_SAMPLE) {
            let subscriptions =
                attributed_subscriptions(&mut conn, tenant_id, customer_id, false).await?;
            // What each subscription for a resource would bill: its own events, over the stretches
            // of the window it prices the metric. Never a difference of totals, wrong for non-sums.
            for subscription in subscriptions
                .iter()
                .filter(|s| s.resource.is_some() && s.claims_now())
            {
                let scope = scopes_for(
                    subscription.id,
                    subscription.created_at,
                    subscription.resource.clone(),
                    &subscriptions,
                )
                .scope_of(&metric);
                if scope.is_nothing() {
                    continue;
                }
                for stretch in stretches(subscription, metric.id, &period) {
                    preview.resource_subscriptions += self
                        .scoped_total(tenant_id, customer_id, &metric, scope.clone(), &stretch)
                        .await?;
                }
            }
            let unclaimed = self
                .scoped_total(
                    tenant_id,
                    customer_id,
                    &metric,
                    unclaimed_scope(&subscriptions, &metric),
                    &period,
                )
                .await?;
            let customer_priced = subscriptions.iter().any(|s| {
                s.resource.is_none()
                    && s.spans.iter().any(|span| {
                        span.metric_id == metric.id
                            && span.from < period.end
                            && span.until.is_none_or(|end| end > period.start)
                    })
            });
            if customer_priced {
                preview.customer_subscriptions += unclaimed;
            } else {
                preview.not_billed += unclaimed;
            }
        }
        Ok(preview)
    }

    async fn scoped_total(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        metric: &BillableMetric,
        scope: crate::domain::UsageScope,
        period: &UsagePeriod,
    ) -> StoreResult<Decimal> {
        let mut scoped = metric.clone();
        scoped.usage_scope = scope;
        self.services
            .usage_client
            .fetch_total_usage(&tenant_id, &customer_id, &scoped, period.clone())
            .await
    }
}
