use crate::StoreResult;
use crate::domain::Price;
use crate::errors::StoreError;
use crate::store::Store;
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

        // A price set for one subscription or plan only is not offered from the catalog.
        let roots: Vec<PriceRow> = PriceRow::list_by_product_id(&mut conn, product_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .filter(|root| root.catalog)
            .collect();
        let ids: Vec<PriceId> = roots.iter().map(|root| root.id).collect();
        let successors = PriceRow::successors_of(&mut conn, &ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Each lineage under its root's id, at its latest rates: what the next edit starts from.
        roots
            .into_iter()
            .map(|root| {
                let id = root.id;
                let tip = successors
                    .iter()
                    .rfind(|row| row.supersedes == Some(id))
                    .cloned()
                    .unwrap_or(root);
                Price::try_from(tip).map(|price| Price { id, ..price })
            })
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

        rows.into_iter().map(Price::try_from).collect()
    }
}
