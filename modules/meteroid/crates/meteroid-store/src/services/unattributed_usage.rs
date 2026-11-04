use crate::StoreResult;
use crate::domain::{
    BillableMetric, SubscriptionDetails, UnattributedUsage, UsagePeriod, unattributed_spans,
    unfiltered_overlaps,
};
use crate::errors::StoreError;
use crate::repositories::subscriptions::usage_attribution::attributed_subscriptions;
use crate::services::ServicesEdge;
use chrono::{Duration, NaiveDateTime};
use common_domain::ids::{BillableMetricId, CustomerId, SubscriptionId, TenantId};
use diesel_models::billable_metrics::BillableMetricRow;
use error_stack::{Report, bail};
use itertools::Itertools;
use rust_decimal::Decimal;
use std::collections::HashMap;

const MAX_RANGE: Duration = Duration::days(366);

impl ServicesEdge {
    /// The customer's subscriptions that would bill the same events as `details` (a subscription
    /// about to be created): both without a usage filter, sharing metrics over their lives.
    pub async fn unfiltered_overlaps(
        &self,
        tenant_id: TenantId,
        details: &SubscriptionDetails,
    ) -> StoreResult<Vec<(SubscriptionId, Vec<BillableMetricId>)>> {
        let subscription = &details.subscription;
        if subscription.usage_filter.is_some() {
            return Ok(Vec::new());
        }
        let metric_ids = details
            .price_components
            .iter()
            .map(|component| &component.fee)
            .chain(details.add_ons.iter().map(|add_on| &add_on.fee))
            .filter_map(|fee| fee.metric_id())
            .unique()
            .collect_vec();
        if metric_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.store.get_conn().await?;
        let siblings =
            attributed_subscriptions(&mut conn, tenant_id, subscription.customer_id, false).await?;
        Ok(unfiltered_overlaps(
            &siblings,
            &metric_ids,
            subscription.start_date.and_time(chrono::NaiveTime::MIN),
            subscription
                .end_date
                .map(|end| end.and_time(chrono::NaiveTime::MIN)),
        ))
    }

    /// The customer's usage over `[from, to)` that no subscription bills: it matches none of the
    /// usage filters of the subscriptions pricing its metric, and none of them is unfiltered.
    pub async fn unattributed_usage(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        from: NaiveDateTime,
        to: NaiveDateTime,
    ) -> StoreResult<Vec<UnattributedUsage>> {
        if from >= to {
            bail!(StoreError::InvalidArgument(
                "`from` must be before `to`".to_string()
            ));
        }
        if to - from > MAX_RANGE {
            bail!(StoreError::InvalidArgument(
                "the range cannot exceed 366 days".to_string()
            ));
        }
        let (spans, metrics, subscriptions) = {
            let mut conn = self.store.get_conn().await?;
            let subscriptions =
                attributed_subscriptions(&mut conn, tenant_id, customer_id, false).await?;
            let spans = unattributed_spans(&subscriptions, from, to);
            let metric_ids = spans
                .iter()
                .map(|span| span.metric_id)
                .unique()
                .collect_vec();
            let metrics = if metric_ids.is_empty() {
                HashMap::new()
            } else {
                BillableMetricRow::get_by_ids(&mut conn, &metric_ids, &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into_iter()
                    .map(|row| {
                        let metric: BillableMetric = row.try_into()?;
                        Ok((metric.id, metric))
                    })
                    .collect::<StoreResult<HashMap<_, _>>>()?
            };
            (spans, metrics, subscriptions)
        };

        let mut usage = Vec::new();
        for span in spans {
            let Some(metric) = metrics.get(&span.metric_id) else {
                continue;
            };
            let mut scoped = metric.clone();
            scoped.usage_scope = span.scope;
            let quantity = self
                .services
                .usage_client
                .fetch_total_usage(
                    &tenant_id,
                    &customer_id,
                    &scoped,
                    UsagePeriod {
                        start: span.from,
                        end: span.until,
                    },
                )
                .await?;
            if quantity != Decimal::ZERO {
                let subscription_ids = subscriptions
                    .iter()
                    .filter(|subscription| {
                        subscription.spans.iter().any(|priced| {
                            priced.metric_id == span.metric_id
                                && priced.from < span.until
                                && priced.until.is_none_or(|until| until > span.from)
                        })
                    })
                    .map(|subscription| subscription.id)
                    .collect();
                usage.push(UnattributedUsage {
                    metric: metric.clone(),
                    from: span.from,
                    until: span.until,
                    quantity,
                    subscription_ids,
                });
            }
        }
        Ok(usage)
    }
}
