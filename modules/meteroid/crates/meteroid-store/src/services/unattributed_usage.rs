use crate::StoreResult;
use crate::domain::{
    BillableMetric, SaleWarnings, SubscriptionDetails, UnattributedUsage, UsagePeriod,
    billing_overlaps, unattributed_spans, unclaimed_scope,
};
use crate::errors::StoreError;
use crate::repositories::subscriptions::usage_attribution::{
    attributed_resources, attributed_subscriptions, sale_warnings,
};
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
    /// The customer's subscriptions for the same resource (or the customer) as `details`, a
    /// subscription about to be created, sharing metrics over their lives.
    pub async fn billing_overlaps(
        &self,
        tenant_id: TenantId,
        details: &SubscriptionDetails,
    ) -> StoreResult<Vec<(SubscriptionId, Vec<BillableMetricId>)>> {
        let subscription = &details.subscription;
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
        let resource = match subscription.resource_id {
            Some(id) => attributed_resources(&mut conn, tenant_id, &[id])
                .await?
                .remove(&id),
            None => None,
        };
        Ok(billing_overlaps(
            &siblings,
            resource.as_ref(),
            &metric_ids,
            subscription.start_date.and_time(chrono::NaiveTime::MIN),
            subscription
                .end_date
                .map(|end| end.and_time(chrono::NaiveTime::MIN)),
        ))
    }

    /// What creating `details`, a subscription about to be created, is accepted with (§3.5, §3.6).
    pub async fn sale_warnings(
        &self,
        tenant_id: TenantId,
        details: &SubscriptionDetails,
    ) -> StoreResult<SaleWarnings> {
        let subscription = &details.subscription;
        let metric_ids = details
            .price_components
            .iter()
            .map(|component| &component.fee)
            .chain(details.add_ons.iter().map(|add_on| &add_on.fee))
            .filter_map(|fee| fee.metric_id())
            .unique()
            .collect_vec();
        let mut conn = self.store.get_conn().await?;
        let resource_type_id = match subscription.resource_id {
            Some(id) => attributed_resources(&mut conn, tenant_id, &[id])
                .await?
                .remove(&id)
                .map(|resource| resource.type_id),
            None => None,
        };
        sale_warnings(
            &mut conn,
            tenant_id,
            subscription.plan_version_id,
            resource_type_id,
            &metric_ids,
        )
        .await
    }

    /// The customer's usage over `[from, to)` that no subscription bills: no subscription for its
    /// resource prices its metric, and no customer-level one does.
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
            scoped.usage_scope = unclaimed_scope(&subscriptions, metric);
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
                        subscription.resource.is_some()
                            && subscription.claims_now()
                            && subscription.spans.iter().any(|priced| {
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
