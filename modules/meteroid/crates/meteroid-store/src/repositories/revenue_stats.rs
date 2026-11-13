//! The home dashboard figures: MRR history and breakdown, billed net revenue, the MRR log and the
//! top customers. Core reads them from Postgres; the Intelligence module serves them from its
//! analytics store.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::{Datelike, NaiveDate};
use common_domain::ids::{CustomerId, PlanId, TenantId};
use diesel_models::enums::MrrMovementType as DbMovementType;
use diesel_models::revenue_stats::{CustomerNameRow, MrrLogRow, MrrMovementDayRow, RevenueDayRow};
use error_stack::Report;
use uuid::Uuid;

use crate::domain::fx::FxTable;
use crate::domain::stats::{
    MrrLogEntry, MrrMovementType, MrrOpeningBalanceRow, MrrTimeseriesPointRow, MrrTimeseriesRows,
    RevenueByCustomer, RevenueDailyPointRow,
};
use crate::errors::StoreError;
use crate::repositories::historical_rates::HistoricalRatesInterface;
use crate::{Store, StoreResult};

#[async_trait::async_trait]
pub trait RevenueStats: Send + Sync {
    /// Opening MRR before `from` and per-day movements over `[from, to]`, in
    /// `reporting_currency` at each movement date's rate.
    async fn mrr_timeseries(
        &self,
        tenant_id: TenantId,
        from: NaiveDate,
        to: NaiveDate,
        plan_ids: &[PlanId],
        split_by_plan: bool,
        reporting_currency: &str,
    ) -> StoreResult<MrrTimeseriesRows>;

    /// Per-day billed revenue over `[from, to]`, in `reporting_currency`: issued invoices (voided
    /// ones too, their void note reverses them) net of discounts and tax, less what purchased
    /// credit paid; finalized credit notes subtract on their finalization date.
    async fn revenue_daily(
        &self,
        tenant_id: TenantId,
        from: NaiveDate,
        to: NaiveDate,
        plan_ids: &[PlanId],
        reporting_currency: &str,
    ) -> StoreResult<Vec<RevenueDailyPointRow>>;

    /// The latest MRR movements (applying within `window` when set), each in its own currency.
    async fn mrr_log(
        &self,
        tenant_id: TenantId,
        limit: u32,
        window: Option<(NaiveDate, NaiveDate)>,
    ) -> StoreResult<Vec<MrrLogEntry>>;

    /// Customers by all-time billed net revenue, with the year to date, in `reporting_currency`.
    async fn top_revenue_by_customer(
        &self,
        tenant_id: TenantId,
        limit: u32,
        reporting_currency: &str,
    ) -> StoreResult<Vec<RevenueByCustomer>>;
}

pub fn postgres_revenue_stats(store: &Store) -> Arc<dyn RevenueStats> {
    Arc::new(PgRevenueStats {
        store: store.clone(),
    })
}

pub struct PgRevenueStats {
    store: Store,
}

impl PgRevenueStats {
    /// One conversion table per date, from the latest rates on or before it. A date without any
    /// rate converts only the reporting currency itself.
    async fn fx_tables(
        &self,
        dates: impl IntoIterator<Item = NaiveDate>,
        reporting_currency: &str,
    ) -> StoreResult<HashMap<NaiveDate, FxTable>> {
        let dates: Vec<NaiveDate> = dates.into_iter().collect();
        let rates = self.store.find_historical_rates_by_dates(&dates).await?;
        Ok(dates
            .into_iter()
            .map(|date| {
                let table = match rates.get(&date) {
                    Some(r) => FxTable::from_usd_rates(reporting_currency, &r.rates),
                    None => FxTable::identity(reporting_currency),
                };
                (date, table)
            })
            .collect())
    }
}

fn convertible(tables: &HashMap<NaiveDate, FxTable>, date: NaiveDate, currency: &str) -> bool {
    let known = tables
        .get(&date)
        .is_some_and(|t| t.factor(currency).is_some());
    if !known {
        log::warn!("revenue stats: no {currency} rate on {date}; amounts left out");
    }
    known
}

fn convert(
    tables: &HashMap<NaiveDate, FxTable>,
    date: NaiveDate,
    amount: i64,
    currency: &str,
) -> i64 {
    tables
        .get(&date)
        .and_then(|t| t.convert(amount, currency))
        .unwrap_or_default()
}

fn plan_uuids(plan_ids: &[PlanId]) -> Vec<Uuid> {
    plan_ids.iter().map(|p| **p).collect()
}

#[async_trait::async_trait]
impl RevenueStats for PgRevenueStats {
    async fn mrr_timeseries(
        &self,
        tenant_id: TenantId,
        from: NaiveDate,
        to: NaiveDate,
        plan_ids: &[PlanId],
        split_by_plan: bool,
        reporting_currency: &str,
    ) -> StoreResult<MrrTimeseriesRows> {
        let plans = plan_uuids(plan_ids);
        let mut conn = self.store.get_conn().await?;
        let before = MrrMovementDayRow::list(
            &mut conn,
            tenant_id,
            None,
            from,
            false,
            &plans,
            split_by_plan,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let within = MrrMovementDayRow::list(
            &mut conn,
            tenant_id,
            Some(from),
            to,
            true,
            &plans,
            split_by_plan,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);
        let tables = self
            .fx_tables(
                before.iter().chain(&within).map(|r| r.date),
                reporting_currency,
            )
            .await?;

        let mut opening: BTreeMap<Option<Uuid>, i64> = BTreeMap::new();
        for row in before
            .iter()
            .filter(|r| convertible(&tables, r.date, &r.currency))
        {
            *opening.entry(row.plan_id).or_default() +=
                convert(&tables, row.date, row.net, &row.currency);
        }
        let mut points: BTreeMap<(NaiveDate, Option<Uuid>), MrrTimeseriesPointRow> =
            BTreeMap::new();
        for row in within
            .iter()
            .filter(|r| convertible(&tables, r.date, &r.currency))
        {
            let c = |amount: i64| convert(&tables, row.date, amount, &row.currency);
            let point =
                points
                    .entry((row.date, row.plan_id))
                    .or_insert_with(|| MrrTimeseriesPointRow {
                        date: row.date,
                        plan_id: row.plan_id.map(PlanId::from),
                        ..Default::default()
                    });
            point.net_mrr_cents += c(row.net);
            point.new_business_cents += c(row.new_business);
            point.new_business_count += row.new_business_count;
            point.expansion_cents += c(row.expansion);
            point.expansion_count += row.expansion_count;
            point.contraction_cents += c(row.contraction);
            point.contraction_count += row.contraction_count;
            point.churn_cents += c(row.churn);
            point.churn_count += row.churn_count;
            point.reactivation_cents += c(row.reactivation);
            point.reactivation_count += row.reactivation_count;
        }
        Ok(MrrTimeseriesRows {
            opening: opening
                .into_iter()
                .map(|(plan_id, mrr_cents)| MrrOpeningBalanceRow {
                    plan_id: plan_id.map(PlanId::from),
                    mrr_cents,
                })
                .collect(),
            points: points.into_values().collect(),
        })
    }

    async fn revenue_daily(
        &self,
        tenant_id: TenantId,
        from: NaiveDate,
        to: NaiveDate,
        plan_ids: &[PlanId],
        reporting_currency: &str,
    ) -> StoreResult<Vec<RevenueDailyPointRow>> {
        let mut conn = self.store.get_conn().await?;
        let rows = RevenueDayRow::list(
            &mut conn,
            tenant_id,
            Some((from, to)),
            &plan_uuids(plan_ids),
            false,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);
        let tables = self
            .fx_tables(rows.iter().map(|r| r.date), reporting_currency)
            .await?;
        let mut days: BTreeMap<NaiveDate, i64> = BTreeMap::new();
        for row in rows
            .iter()
            .filter(|r| convertible(&tables, r.date, &r.currency))
        {
            *days.entry(row.date).or_default() +=
                convert(&tables, row.date, row.net, &row.currency);
        }
        Ok(days
            .into_iter()
            .map(|(date, net_revenue_cents)| RevenueDailyPointRow {
                date,
                net_revenue_cents,
            })
            .collect())
    }

    async fn mrr_log(
        &self,
        tenant_id: TenantId,
        limit: u32,
        window: Option<(NaiveDate, NaiveDate)>,
    ) -> StoreResult<Vec<MrrLogEntry>> {
        let mut conn = self.store.get_conn().await?;
        let rows = MrrLogRow::list(&mut conn, tenant_id, i64::from(limit), window)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(rows
            .into_iter()
            .map(|row| MrrLogEntry {
                created_at: row.created_at,
                applies_to: row.applies_to,
                customer_id: CustomerId::from(row.customer_id).to_string(),
                customer_name: row.customer_name,
                subscription_id: common_domain::ids::SubscriptionId::from(row.subscription_id)
                    .to_string(),
                plan_name: row.plan_name,
                description: row.description,
                mrr_type: match row.movement_type {
                    DbMovementType::NewBusiness => MrrMovementType::NewBusiness,
                    DbMovementType::Expansion => MrrMovementType::Expansion,
                    DbMovementType::Contraction => MrrMovementType::Contraction,
                    DbMovementType::Churn => MrrMovementType::Churn,
                    DbMovementType::Reactivation => MrrMovementType::Reactivation,
                },
                mrr_change: row.net_mrr_change,
                currency: row.currency,
            })
            .collect())
    }

    async fn top_revenue_by_customer(
        &self,
        tenant_id: TenantId,
        limit: u32,
        reporting_currency: &str,
    ) -> StoreResult<Vec<RevenueByCustomer>> {
        let mut conn = self.store.get_conn().await?;
        let rows = RevenueDayRow::list(&mut conn, tenant_id, None, &[], true)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let tables = self
            .fx_tables(rows.iter().map(|r| r.date), reporting_currency)
            .await?;
        let year = common_domain::clock::now().date_naive().year();
        let mut totals: HashMap<Uuid, (i64, i64)> = HashMap::new();
        for row in rows
            .iter()
            .filter(|r| convertible(&tables, r.date, &r.currency))
        {
            let Some(customer_id) = row.customer_id else {
                continue;
            };
            let amount = convert(&tables, row.date, row.net, &row.currency);
            let total = totals.entry(customer_id).or_default();
            total.1 += amount;
            if row.date.year() == year {
                total.0 += amount;
            }
        }
        let mut ranked: Vec<(Uuid, (i64, i64))> = totals.into_iter().collect();
        ranked.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(&b.0)));
        ranked.truncate(limit as usize);
        let ids: Vec<Uuid> = ranked.iter().map(|(id, _)| *id).collect();
        let names: HashMap<Uuid, String> = CustomerNameRow::list(&mut conn, tenant_id, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|row| (row.id, row.name))
            .collect();
        Ok(ranked
            .into_iter()
            .map(|(id, (revenue_ytd, revenue_all_time))| RevenueByCustomer {
                customer_name: names.get(&id).cloned().unwrap_or_default(),
                customer_id: CustomerId::from(id),
                revenue_ytd,
                revenue_all_time,
                currency: reporting_currency.to_string(),
            })
            .collect())
    }
}
