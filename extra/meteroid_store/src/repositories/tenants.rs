use crate::domain::Tenant;
use crate::errors;
use crate::errors::db_error_to_store;
use crate::store::Store;
use crate::{domain, StoreResult};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait TenantInterface {
    async fn insert_tenant(&self, tenant: domain::TenantNew) -> StoreResult<domain::Tenant>;
    async fn find_tenant_by_id(&self, tenant_id: Uuid) -> StoreResult<domain::Tenant>;
}

#[async_trait::async_trait]
impl TenantInterface for Store {
    async fn insert_tenant(&self, tenant: domain::TenantNew) -> StoreResult<domain::Tenant> {
        let mut conn = self.get_conn().await?;

        let insertable_tenant: diesel_models::tenants::TenantNew = tenant.into();

        let res = insertable_tenant
            .insert(&mut conn)
            .await
            .map_err(db_error_to_store)
            .map(Into::into)?;

        Ok(res)
    }

    async fn find_tenant_by_id(&self, tenant_id: Uuid) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        let tenant = diesel_models::tenants::Tenant::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(db_error_to_store)
            .map(Into::into)?;

        Ok(tenant)
    }
}
