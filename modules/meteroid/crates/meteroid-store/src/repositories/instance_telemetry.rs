//! Storage for the anonymous instance report.

use chrono::{NaiveDate, NaiveDateTime};
use diesel_models::instance_telemetry::{InstanceTelemetryRow, InstanceUsageRow};
use error_stack::Report;

use crate::errors::StoreError;
use crate::repositories::historical_rates::convert_currency;
use crate::repositories::production_revenue::ProductionRevenueInterface;
use crate::{Store, StoreResult};

#[derive(Debug, Clone)]
pub struct InstanceTelemetry {
    pub instance_id: uuid::Uuid,
    pub installed_at: NaiveDateTime,
    pub first_version: String,
    pub last_sent_at: Option<NaiveDateTime>,
}

impl From<InstanceTelemetryRow> for InstanceTelemetry {
    fn from(row: InstanceTelemetryRow) -> Self {
        Self {
            instance_id: row.instance_id,
            installed_at: row.installed_at,
            first_version: row.first_version,
            last_sent_at: row.last_sent_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceUsage {
    pub organizations: u64,
    pub tenants_live: u64,
    pub tenants_other: u64,
    pub users: u64,
    pub customers: u64,
    pub subscriptions_active: u64,
    pub plans_published: u64,
    pub billable_metrics: u64,
    pub invoices_finalized_30d: u64,
    pub currencies: Vec<String>,
    pub integrations: Vec<String>,
    pub postgres_major: u32,
    /// Production revenue of the last 30 closed days, in USD cents; currencies without a rate are left out.
    pub billed_30d_usd_cents: i64,
}

#[async_trait::async_trait]
pub trait InstanceTelemetryInterface {
    /// The telemetry identity, created with a random id on first use.
    async fn instance_telemetry(&self, version: &str) -> StoreResult<InstanceTelemetry>;
    async fn record_instance_telemetry_sent(&self) -> StoreResult<()>;
    async fn instance_usage(&self, today: NaiveDate) -> StoreResult<InstanceUsage>;
}

fn count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

#[async_trait::async_trait]
impl InstanceTelemetryInterface for Store {
    async fn instance_telemetry(&self, version: &str) -> StoreResult<InstanceTelemetry> {
        let mut conn = self.get_conn().await?;
        if let Some(row) = InstanceTelemetryRow::get(&mut conn).await? {
            return Ok(row.into());
        }
        InstanceTelemetryRow::get_or_create(&mut conn, uuid::Uuid::new_v4(), version)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    async fn record_instance_telemetry_sent(&self) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        InstanceTelemetryRow::record_sent(&mut conn, chrono::Utc::now().naive_utc())
            .await
            .map_err(Into::into)
    }

    async fn instance_usage(&self, today: NaiveDate) -> StoreResult<InstanceUsage> {
        let from = today - chrono::Days::new(30);
        let revenue = self.daily_production_revenue(from, today).await?;
        let mut conn = self.get_conn().await?;
        let row = InstanceUsageRow::query(&mut conn).await?;

        let mut by_currency: std::collections::BTreeMap<String, i64> = Default::default();
        for day in revenue {
            *by_currency.entry(day.currency).or_default() += day.amount_cents;
        }
        let mut billed_30d_usd_cents: i64 = 0;
        for (currency, amount) in by_currency {
            match convert_currency(&mut conn, amount, &currency, "USD").await {
                Ok(usd) => billed_30d_usd_cents = billed_30d_usd_cents.saturating_add(usd),
                Err(e) => log::warn!("telemetry: {currency} revenue left out: {e:?}"),
            }
        }

        Ok(InstanceUsage {
            organizations: count(row.organizations),
            tenants_live: count(row.tenants_live),
            tenants_other: count(row.tenants_other),
            users: count(row.users),
            customers: count(row.customers),
            subscriptions_active: count(row.subscriptions_active),
            plans_published: count(row.plans_published),
            billable_metrics: count(row.billable_metrics),
            invoices_finalized_30d: count(row.invoices_finalized_30d),
            currencies: row.currencies,
            integrations: row.integrations,
            postgres_major: u32::try_from(row.postgres_major)
                .map_err(|_| Report::new(StoreError::InvalidArgument("postgres version".into())))?,
            billed_30d_usd_cents,
        })
    }
}
