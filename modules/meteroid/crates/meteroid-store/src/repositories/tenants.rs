use crate::domain::{Tenant, TenantNew};
use error_stack::Report;

use crate::store::Store;
use crate::{errors, StoreResult};
use diesel_models::organizations::OrganizationRow;
use diesel_models::tenants::{TenantRow, TenantRowNew};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait TenantInterface {
    async fn insert_tenant(&self, tenant: TenantNew) -> StoreResult<Tenant>;
    async fn find_tenant_by_id(&self, tenant_id: Uuid) -> StoreResult<Tenant>;
    async fn find_tenant_by_slug_and_organization_slug(&self, slug: String, organization_slug: String) -> StoreResult<Tenant> ;
    async fn list_tenants_by_organization_id(&self, organization_id: Uuid) -> StoreResult<Vec<Tenant>>;
}

#[async_trait::async_trait]
impl TenantInterface for Store {
    async fn insert_tenant(&self, tenant: TenantNew) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        // TODO no, it can only be for org I guess ?? Also, insert an invoicing entity
        let insertable_tenant: TenantRowNew = tenant.into();

        insertable_tenant
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_tenant_by_id(&self, tenant_id: Uuid) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        TenantRow::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_tenant_by_slug_and_organization_slug(&self, slug: String, organization_slug: String) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        TenantRow::find_by_slug_and_organization_slug(&mut conn, slug, organization_slug)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_tenants_by_organization_id(&self, organization_id: Uuid) -> StoreResult<Vec<Tenant>> {
        let mut conn = self.get_conn().await?;

        TenantRow::list_by_organization_id(&mut conn, organization_id)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }
}
