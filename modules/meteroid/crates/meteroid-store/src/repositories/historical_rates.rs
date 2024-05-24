use crate::domain::historical_rates::{HistoricalRatesFromUsd, HistoricalRatesFromUsdNew};
use crate::{Store, StoreResult};

#[async_trait::async_trait]
pub trait HistoricalRatesInterface {
    async fn create_historical_rate_from_usd(
        &self,
        rate: HistoricalRatesFromUsdNew,
    ) -> StoreResult<HistoricalRatesFromUsd>;
}

#[async_trait::async_trait]
impl HistoricalRatesInterface for Store {
    async fn create_historical_rate_from_usd(
        &self,
        rate: HistoricalRatesFromUsdNew,
    ) -> StoreResult<HistoricalRatesFromUsd> {
        let mut conn = self.get_conn().await?;

        let insertable: diesel_models::historical_rates_from_usd::HistoricalRatesFromUsdNew =
            rate.try_into()?;

        insertable
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }
}
