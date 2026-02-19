use crate::domain::Price;
use crate::errors::StoreError;
use crate::store::Store;
use crate::StoreResult;
use common_domain::ids::{PriceId, ProductId, TenantId};
use diesel_models::prices::PriceRow;
use error_stack::Report;

#[async_trait::async_trait]
pub trait PriceInterface {
    async fn list_prices_by_product_id(
        &self,
        product_id: ProductId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Price>>;
    async fn list_prices_by_ids(
        &self,
        ids: &[PriceId],
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Price>>;
}

#[async_trait::async_trait]
impl PriceInterface for Store {
    async fn list_prices_by_product_id(
        &self,
        product_id: ProductId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Price>> {
        let mut conn = self.get_conn().await?;

        let rows = PriceRow::list_by_product_id(&mut conn, product_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(|row| Price::try_from(row).map_err(Into::<Report<StoreError>>::into))
            .collect()
    }

    async fn list_prices_by_ids(
        &self,
        ids: &[PriceId],
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Price>> {
        let mut conn = self.get_conn().await?;

        let rows = PriceRow::list_by_ids(&mut conn, ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(|row| Price::try_from(row).map_err(Into::<Report<StoreError>>::into))
            .collect()
    }
}
