use crate::store::{PgConn, Store, StoreInternal};
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

impl StoreInternal {
    pub async fn insert_product_family(
        &self,
        conn: &mut PgConn,
        product_family: domain::ProductFamilyNew,
    ) -> StoreResult<domain::ProductFamily> {
        let insertable_product_family: ProductFamilyRowNew = product_family.into();

        insertable_product_family
            .insert(conn)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }
}

#[async_trait::async_trait]
impl ProductFamilyInterface for Store {
    async fn insert_product_family(
        &self,
        product_family: domain::ProductFamilyNew,
        actor: Option<Uuid>,
    ) -> StoreResult<domain::ProductFamily> {
        let mut conn = self.get_conn().await?;

        let res = self
            .internal
            .insert_product_family(&mut conn, product_family)
            .await?;

        let _ = self
            .eventbus
            .publish(Event::product_family_created(actor, res.id, res.tenant_id))
            .await;

        Ok(res)
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
