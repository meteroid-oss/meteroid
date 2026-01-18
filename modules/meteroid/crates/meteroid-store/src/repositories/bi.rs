use crate::domain::historical_rates::HistoricalRatesFromUsd;
use crate::repositories::historical_rates::get_historical_rate_from_usd_by_date_cached;
use crate::{Store, StoreResult};
use chrono::{Datelike, NaiveDate, NaiveDateTime};
use common_domain::ids::{BaseId, CustomerId, PlanVersionId, TenantId};
use diesel_models::bi::{BiCustomerYtdSummaryRow, BiDeltaMrrDailyRow, BiRevenueDailyRow};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Input data for recording invoice revenue
pub struct InvoiceRevenueInput {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: Option<PlanVersionId>,
    pub currency: String,
    pub amount_cents: i64,
    pub finalized_at: NaiveDateTime,
}

/// Input data for recording credit note (negative revenue)
pub struct CreditNoteRevenueInput {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: Option<PlanVersionId>,
    pub currency: String,
    pub refunded_amount_cents: i64,
    pub finalized_at: NaiveDateTime,
}

#[async_trait::async_trait]
pub trait BiAggregationInterface {
    /// Record invoice revenue in BI tables (bi_revenue_daily and bi_customer_ytd_summary)
    async fn record_invoice_revenue(&self, input: InvoiceRevenueInput) -> StoreResult<()>;

    /// Record credit note (as negative revenue) in BI tables
    async fn record_credit_note_revenue(&self, input: CreditNoteRevenueInput) -> StoreResult<()>;

    /// Update USD values in BI tables for a given date using new rates
    /// This is called by the currency rates worker when new rates are fetched
    async fn update_bi_usd_values(
        &self,
        date: NaiveDate,
        rates: &BTreeMap<String, f32>,
        historical_rate_id: Uuid,
    ) -> StoreResult<(usize, usize)>;
}

#[async_trait::async_trait]
impl BiAggregationInterface for Store {
    async fn record_invoice_revenue(&self, input: InvoiceRevenueInput) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let revenue_date = input.finalized_at.date();
        let revenue_year = revenue_date.year();

        // Get historical rate for USD conversion
        let rates = get_historical_rate_from_usd_by_date_cached(&mut conn, revenue_date).await?;

        let (historical_rate_id, amount_cents_usd) =
            convert_to_usd(&input.currency, input.amount_cents, rates)?;

        // Upsert to bi_revenue_daily
        let revenue_row = BiRevenueDailyRow {
            tenant_id: input.tenant_id,
            plan_version_id: input.plan_version_id.map(|id| id.as_uuid()),
            currency: input.currency.clone(),
            revenue_date,
            net_revenue_cents: input.amount_cents,
            historical_rate_id,
            net_revenue_cents_usd: amount_cents_usd,
            id: uuid::Uuid::now_v7(),
        };

        BiRevenueDailyRow::upsert(&mut conn, revenue_row).await?;

        // Upsert to bi_customer_ytd_summary
        let ytd_row = BiCustomerYtdSummaryRow {
            tenant_id: input.tenant_id,
            customer_id: input.customer_id,
            revenue_year,
            currency: input.currency,
            total_revenue_cents: input.amount_cents,
        };

        BiCustomerYtdSummaryRow::upsert(&mut conn, ytd_row).await?;

        Ok(())
    }

    async fn record_credit_note_revenue(&self, input: CreditNoteRevenueInput) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let revenue_date = input.finalized_at.date();
        let revenue_year = revenue_date.year();

        // Get historical rate for USD conversion
        let rates = get_historical_rate_from_usd_by_date_cached(&mut conn, revenue_date).await?;

        // Credit notes are negative revenue
        let amount_cents = -input.refunded_amount_cents;

        let (historical_rate_id, amount_cents_usd) =
            convert_to_usd(&input.currency, amount_cents, rates)?;

        // Upsert to bi_revenue_daily (negative amount)
        let revenue_row = BiRevenueDailyRow {
            tenant_id: input.tenant_id,
            plan_version_id: input.plan_version_id.map(|id| id.as_uuid()),
            currency: input.currency.clone(),
            revenue_date,
            net_revenue_cents: amount_cents,
            historical_rate_id,
            net_revenue_cents_usd: amount_cents_usd,
            id: uuid::Uuid::now_v7(),
        };

        BiRevenueDailyRow::upsert(&mut conn, revenue_row).await?;

        // Upsert to bi_customer_ytd_summary (negative amount)
        let ytd_row = BiCustomerYtdSummaryRow {
            tenant_id: input.tenant_id,
            customer_id: input.customer_id,
            revenue_year,
            currency: input.currency,
            total_revenue_cents: amount_cents,
        };

        BiCustomerYtdSummaryRow::upsert(&mut conn, ytd_row).await?;

        Ok(())
    }

    async fn update_bi_usd_values(
        &self,
        date: NaiveDate,
        rates: &BTreeMap<String, f32>,
        historical_rate_id: Uuid,
    ) -> StoreResult<(usize, usize)> {
        let mut conn = self.get_conn().await?;

        // Update bi_delta_mrr_daily USD values
        let mrr_updated = BiDeltaMrrDailyRow::update_usd_values_for_date(
            &mut conn,
            date,
            rates,
            historical_rate_id,
        )
        .await?;

        // Update bi_revenue_daily USD values
        let revenue_updated = BiRevenueDailyRow::update_usd_values_for_date(
            &mut conn,
            date,
            rates,
            historical_rate_id,
        )
        .await?;

        Ok((mrr_updated, revenue_updated))
    }
}

/// Convert an amount to USD using historical rates.
/// Returns (historical_rate_id, amount_in_usd_cents) as Decimal for high precision.
fn convert_to_usd(
    currency: &str,
    amount_cents: i64,
    rates: Option<HistoricalRatesFromUsd>,
) -> StoreResult<(Uuid, Decimal)> {
    let rates = rates.ok_or_else(|| {
        crate::errors::StoreError::ValueNotFound("No historical rates found".to_string())
    })?;

    let rate = rates.rates.get(currency).ok_or_else(|| {
        crate::errors::StoreError::ValueNotFound(format!(
            "No rate found for currency: {}",
            currency
        ))
    })?;

    // The rate is from USD to the currency (e.g., 1 USD = 0.92 EUR means rate = 0.92)
    // To convert FROM the currency TO USD, we divide by the rate
    let rate_decimal = Decimal::from_f32(*rate).ok_or_else(|| {
        crate::errors::StoreError::InvalidArgument(format!(
            "Invalid rate value for currency: {}",
            currency
        ))
    })?;

    // Convert to USD - store with full precision in NUMERIC(20,4)
    let amount_usd = Decimal::from(amount_cents) / rate_decimal;

    Ok((rates.id, amount_usd))
}
