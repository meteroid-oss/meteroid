use chrono::NaiveDate;
use diesel_models::query::production_revenue::daily_production_revenue;

use crate::{Store, StoreResult};

/// Revenue of production tenants finalized on one UTC day, in one currency.
pub struct DailyRevenue {
    pub day: NaiveDate,
    pub currency: String,
    /// Invoices minus credit notes, pre-tax, in minor units.
    pub amount_cents: i64,
    pub invoice_count: u32,
    pub credit_note_count: u32,
}

#[async_trait::async_trait]
pub trait ProductionRevenueInterface {
    /// Revenue finalized on each day of `[from, to)`, by currency; days without any are left out.
    async fn daily_production_revenue(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> StoreResult<Vec<DailyRevenue>>;
}

#[async_trait::async_trait]
impl ProductionRevenueInterface for Store {
    async fn daily_production_revenue(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> StoreResult<Vec<DailyRevenue>> {
        let mut conn = self.get_conn().await?;
        let rows = daily_production_revenue(&mut conn, from, to)
            .await
            .map_err(Into::<error_stack::Report<crate::errors::StoreError>>::into)?;
        Ok(rows
            .into_iter()
            .map(|r| DailyRevenue {
                day: r.day,
                currency: r.currency,
                amount_cents: r.amount_cents,
                invoice_count: u32::try_from(r.invoice_count).unwrap_or(u32::MAX),
                credit_note_count: u32::try_from(r.credit_note_count).unwrap_or(u32::MAX),
            })
            .collect())
    }
}
