//! Usage delivered but not invoiced at an instant (ADR-0014 §4): each usage period open across it,
//! priced by the path its period invoice is drafted on, metering read only up to the instant.

use std::collections::HashSet;
use std::sync::Arc;

use billing_core::evaluate::DocumentSlot;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::{
    BillableMetricId, CustomerId, InvoicingEntityId, ProductId, SubscriptionId, TenantId,
};
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::errors::IntoDbResult;
use diesel_models::invoices::InvoiceRow;
use error_stack::Report;
use rust_decimal::Decimal;

use super::load::load_facts;
use super::usage::plan_usage;
use crate::StoreResult;
use crate::domain::enums::InvoiceType;
use crate::domain::{Invoice, UsagePeriod};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::clients::usage::{
    EventSearchOptions, EventSearchResult, IngestEventsRequest, IngestEventsResult, UsageClient,
    UsageData, WindowedUsageData,
};
use crate::services::invoice_lines::component::is_usage_based_line;
use crate::services::invoice_lines::credit_units::CreditUnitCover;
use crate::services::invoices::upcoming::upcoming_invoice_details_on;
use crate::services::{Services, ServicesEdge};
use crate::store::PgConn;

/// A usage line of a period open at the instant, as its invoice would bill the usage so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnbilledUsage {
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub product_id: Option<ProductId>,
    pub currency: String,
    /// The days the line bills, as its invoice dates them, and that invoice's date.
    pub period: (NaiveDate, NaiveDate),
    pub invoice_date: NaiveDate,
    /// Minor units, net of tax (and of the discounts and promotional credit the line carries).
    pub amount: i64,
}

/// `period` cut at `until`; None when nothing of it is left.
fn capped(period: UsagePeriod, until: NaiveDateTime) -> Option<UsagePeriod> {
    let end = period.end.min(until);
    (end > period.start).then_some(UsagePeriod {
        start: period.start,
        end,
    })
}

/// Metering as it stood at `until`: every read ends there at the latest.
struct UsageUntil {
    inner: Arc<dyn UsageClient>,
    until: NaiveDateTime,
}

impl UsageUntil {
    fn empty(period: UsagePeriod) -> UsageData {
        UsageData {
            data: Vec::new(),
            period,
        }
    }
}

#[async_trait::async_trait]
impl UsageClient for UsageUntil {
    async fn fetch_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &crate::domain::BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        match capped(period.clone(), self.until) {
            Some(period) => {
                self.inner
                    .fetch_usage(tenant_id, customer_id, metric, period)
                    .await
            }
            None => Ok(Self::empty(period)),
        }
    }

    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &crate::domain::BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        match capped(period.clone(), self.until) {
            Some(period) => {
                self.inner
                    .fetch_usage_grouped_by(tenant_id, customer_id, metric, group_by, period)
                    .await
            }
            None => Ok(Self::empty(period)),
        }
    }

    async fn fetch_total_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &crate::domain::BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        match capped(period, self.until) {
            Some(period) => {
                self.inner
                    .fetch_total_usage(tenant_id, customer_id, metric, period)
                    .await
            }
            None => Ok(Decimal::ZERO),
        }
    }

    async fn fetch_windowed_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &crate::domain::BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        match capped(period.clone(), self.until) {
            Some(period) => {
                self.inner
                    .fetch_windowed_usage(tenant_id, customer_id, metric, period)
                    .await
            }
            None => Ok(WindowedUsageData {
                data: Vec::new(),
                period,
            }),
        }
    }

    async fn fetch_usage_summary(
        &self,
        tenant_id: &TenantId,
        customer_id: Option<&CustomerId>,
        metric: &crate::domain::BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        match capped(period.clone(), self.until) {
            Some(period) => {
                self.inner
                    .fetch_usage_summary(tenant_id, customer_id, metric, period)
                    .await
            }
            None => Ok(Self::empty(period)),
        }
    }

    async fn search_events(
        &self,
        tenant_id: &TenantId,
        mut options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        let until = self.until.and_utc().timestamp();
        let to = options
            .to
            .as_ref()
            .map_or(until, |to| to.seconds.min(until));
        options.to = Some(prost_types::Timestamp {
            seconds: to,
            nanos: 0,
        });
        self.inner.search_events(tenant_id, options).await
    }

    async fn ingest_events(
        &self,
        _tenant_id: &TenantId,
        _request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        Err(Report::new(StoreError::InvalidArgument(
            "metering read as of an instant takes no events".to_string(),
        )))
    }
}

impl Services {
    /// The subscriptions of the entity's customers that may hold a usage period open at `at`.
    async fn subscriptions_open_at(
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity_id: InvoicingEntityId,
        at: DateTime<Utc>,
    ) -> StoreResult<Vec<SubscriptionId>> {
        use diesel_models::schema::{customer, subscription};
        let day = at.date_naive();
        subscription::table
            .inner_join(customer::table)
            .filter(subscription::tenant_id.eq(tenant_id))
            .filter(customer::invoicing_entity_id.eq(entity_id))
            .filter(subscription::start_date.lt(day))
            .filter(
                subscription::end_date
                    .is_null()
                    .or(subscription::end_date.gt(day)),
            )
            .filter(
                subscription::terminated_at
                    .is_null()
                    .or(subscription::terminated_at.gt(at.naive_utc())),
            )
            .order(subscription::id.asc())
            .select(subscription::id)
            .load(conn)
            .await
            .into_db_result()
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn unbilled_usage(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity_id: InvoicingEntityId,
        at: DateTime<Utc>,
    ) -> StoreResult<Vec<UnbilledUsage>> {
        let day = at.date_naive();
        let as_of = self.reading_usage(Arc::new(UsageUntil {
            inner: self.usage_client.clone(),
            until: at.naive_utc(),
        }));
        let mut unbilled = Vec::new();
        for subscription_id in Self::subscriptions_open_at(conn, tenant_id, entity_id, at).await? {
            let loaded = load_facts(conn, tenant_id, subscription_id, day, None).await?;
            let mut open: Vec<(NaiveDate, HashSet<BillableMetricId>)> = Vec::new();
            for need in plan_usage(&loaded, at)? {
                let window = need.read.window;
                let DocumentSlot::Recurring { period_start } = need.document else {
                    continue;
                };
                if window.from >= at || window.until <= at {
                    continue;
                }
                match open.iter_mut().find(|(on, _)| *on == period_start) {
                    Some((_, metrics)) => {
                        metrics.insert(need.read.metric);
                    }
                    None => open.push((period_start, HashSet::from([need.read.metric]))),
                }
            }
            if open.is_empty() {
                continue;
            }
            let details = self
                .store
                .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                .await?;
            for (invoice_date, metrics) in open {
                let invoice: Option<Invoice> = InvoiceRow::find_existing_recurring_invoice(
                    conn,
                    tenant_id,
                    subscription_id,
                    invoice_date,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .filter(|row| {
                    !matches!(
                        row.status,
                        diesel_models::enums::InvoiceStatusEnum::Void
                            | diesel_models::enums::InvoiceStatusEnum::Closed
                    )
                })
                .map(TryInto::try_into)
                .transpose()?;
                let billed = match &invoice {
                    None if details.subscription.current_period_end == Some(invoice_date) => {
                        upcoming_invoice_details_on(conn, &details).await?.0
                    }
                    _ => details.clone(),
                };
                let content = as_of
                    .compute_invoice_from_facts(
                        conn,
                        &invoice_date,
                        &billed,
                        None,
                        invoice.as_ref(),
                        InvoiceType::Recurring,
                        CreditUnitCover::Estimate {
                            invoice_id: invoice.as_ref().map(|invoice| invoice.id),
                        },
                        None,
                    )
                    .await?;
                unbilled.extend(
                    content
                        .invoice_lines
                        .into_iter()
                        .filter(|line| {
                            is_usage_based_line(line)
                                && line.credit_unit.is_none()
                                && line.metric_id.is_some_and(|m| metrics.contains(&m))
                                && line.start_date < day
                                && line.end_date >= day
                                && line.taxable_amount != 0
                        })
                        .map(|line| UnbilledUsage {
                            customer_id: details.subscription.customer_id,
                            subscription_id,
                            product_id: line.product_id,
                            currency: details.subscription.currency.clone(),
                            period: (line.start_date, line.end_date),
                            invoice_date,
                            amount: line.taxable_amount,
                        }),
                );
            }
        }
        Ok(unbilled)
    }
}

impl ServicesEdge {
    /// The usage of each period of the entity's subscriptions open across `at` (started before,
    /// ending after), priced as its invoice would bill it had the period ended at `at`.
    pub async fn unbilled_usage_at(
        &self,
        tenant_id: TenantId,
        entity_id: InvoicingEntityId,
        at: DateTime<Utc>,
    ) -> StoreResult<Vec<UnbilledUsage>> {
        let mut conn = self.store.get_conn().await?;
        self.services
            .unbilled_usage(&mut conn, tenant_id, entity_id, at)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    #[test]
    fn a_read_ends_at_the_instant_at_the_latest() {
        let period = UsagePeriod {
            start: at(15),
            end: at(30),
        };
        assert_eq!(
            capped(period.clone(), at(20)).map(|p| (p.start, p.end)),
            Some((at(15), at(20)))
        );
        assert_eq!(capped(period.clone(), at(30)).map(|p| p.end), Some(at(30)));
        assert!(capped(period, at(15)).is_none());
    }
}
