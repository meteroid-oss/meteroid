use std::collections::HashMap;

use chrono::NaiveDate;
use common_domain::ids::{CustomerId, TenantId};
use error_stack::Report;

use crate::domain::fx::FxTable;
use crate::errors::StoreError;
use crate::repositories::TenantInterface;
use crate::repositories::historical_rates::HistoricalRatesInterface;
use crate::{Store, StoreResult};

/// Conversion into the tenant reporting currency with the latest rates on or before `today`;
/// without any rates row only the reporting currency itself converts.
pub async fn reporting_fx(
    store: &Store,
    tenant_id: TenantId,
    today: NaiveDate,
) -> StoreResult<FxTable> {
    let currency = store.get_reporting_currency_by_tenant_id(tenant_id).await?;
    let rates = store.get_historical_rate_from_usd_by_date(today).await?;
    Ok(match rates {
        Some(r) => FxTable::from_usd_rates(currency.code, &r.rates),
        None => FxTable::identity(currency.code),
    })
}

/// The one per-account MRR (billing-active subscriptions) in `fx`'s target currency.
/// Customers without an active subscription are absent; currencies without a rate are left out.
pub async fn customer_billing_mrr(
    store: &Store,
    tenant_id: TenantId,
    customer_ids: &[CustomerId],
    fx: &FxTable,
) -> StoreResult<HashMap<CustomerId, i64>> {
    let mut conn = store.get_conn().await?;
    let rows = diesel_models::query::account_signals::customer_billing_mrr(
        &mut conn,
        tenant_id,
        customer_ids,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    let mut mrr: HashMap<CustomerId, i64> = HashMap::new();
    for row in rows {
        let converted = fx.convert(row.mrr_cents, &row.currency);
        if converted.is_none() {
            log::warn!(
                "no FX rate from {} to {}: MRR of customer {} left out",
                row.currency,
                fx.target(),
                row.customer_id
            );
        }
        *mrr.entry(row.customer_id).or_default() += converted.unwrap_or(0);
    }
    Ok(mrr)
}
