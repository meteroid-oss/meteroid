use crate::store::Store;
use crate::{domain, StoreResult};

#[async_trait::async_trait]
pub trait ProductFamilyInterface {
    async fn insert_product_family(
        &self,
        product_family: domain::ProductFamilyNew,
    ) -> StoreResult<domain::ProductFamily>;
}

#[async_trait::async_trait]
impl ProductFamilyInterface for Store {
    async fn insert_product_family(
        &self,
        product_family: domain::ProductFamilyNew,
    ) -> StoreResult<domain::ProductFamily> {
        let mut conn = self.get_conn().await?;

        let insertable_product_family: diesel_models::product_families::ProductFamilyNew =
            product_family.into();

        insertable_product_family
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }
}
