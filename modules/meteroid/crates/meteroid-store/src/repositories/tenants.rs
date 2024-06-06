use crate::domain::{OrgTenantNew, Tenant, TenantNew};
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
    async fn find_tenant_by_slug(&self, slug: String) -> StoreResult<Tenant>;
    async fn list_tenants_by_user_id(&self, user_id: Uuid) -> StoreResult<Vec<Tenant>>;
}

#[async_trait::async_trait]
impl TenantInterface for Store {
    async fn insert_tenant(&self, tenant: TenantNew) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        let insertable_tenant: TenantRowNew = match tenant {
            TenantNew::ForOrg(tenant_new) => tenant_new.into(),
            TenantNew::ForUser(tenant_new) => {
                let org = OrganizationRow::find_by_user_id(&mut conn, tenant_new.user_id)
                    .await
                    .map_err(Into::<Report<errors::StoreError>>::into)?;

                let org_tenant = OrgTenantNew {
                    organization_id: org.id,
                    name: tenant_new.name,
                    slug: tenant_new.slug,
                    currency: tenant_new.currency,
                    environment: tenant_new.environment,
                };

                org_tenant.into()
            }
        };

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

    async fn find_tenant_by_slug(&self, slug: String) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        TenantRow::find_by_slug(&mut conn, slug)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_tenants_by_user_id(&self, user_id: Uuid) -> StoreResult<Vec<Tenant>> {
        let mut conn = self.get_conn().await?;

        TenantRow::list_by_user_id(&mut conn, user_id)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }
}
