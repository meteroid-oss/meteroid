use crate::store::Store;
use crate::{domain, StoreResult};
use common_eventbus::Event;
use diesel_models::product_families::{ProductFamilyRow, ProductFamilyRowNew};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait ProductFamilyInterface {
    async fn insert_product_family(
        &self,
        product_family: domain::ProductFamilyNew,
        actor: Option<Uuid>,
    ) -> StoreResult<domain::ProductFamily>;

    async fn list_product_families(
        &self,
        auth_tenant_id: Uuid,
    ) -> StoreResult<Vec<domain::ProductFamily>>;

    async fn find_product_family_by_external_id(
        &self,
        external_id: &str,
        auth_tenant_id: Uuid,
    ) -> StoreResult<domain::ProductFamily>;
}

#[async_trait::async_trait]
impl ProductFamilyInterface for Store {
    async fn insert_product_family(
        &self,
        product_family: domain::ProductFamilyNew,
        actor: Option<Uuid>,
    ) -> StoreResult<domain::ProductFamily> {
        let mut conn = self.get_conn().await?;

        let insertable_product_family: ProductFamilyRowNew = product_family.into();

        let res = insertable_product_family
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into);

        let _ = self
            .eventbus
            .publish(Event::product_family_created(
                actor,
                insertable_product_family.id,
                insertable_product_family.tenant_id,
            ))
            .await;

        res
    }

    async fn list_product_families(
        &self,
        auth_tenant_id: Uuid,
    ) -> StoreResult<Vec<domain::ProductFamily>> {
        let mut conn = self.get_conn().await?;

        ProductFamilyRow::list(&mut conn, auth_tenant_id)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }

    async fn find_product_family_by_external_id(
        &self,
        external_id: &str,
        auth_tenant_id: Uuid,
    ) -> StoreResult<domain::ProductFamily> {
        let mut conn = self.get_conn().await?;

        ProductFamilyRow::find_by_external_id_and_tenant_id(&mut conn, external_id, auth_tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }
}
