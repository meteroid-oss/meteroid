use crate::domain::{OrgTenantNew, Tenant, TenantNew};
use error_stack::Report;

use crate::store::Store;
use crate::{domain, errors, StoreResult};
use diesel_models::organizations::Organization;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait TenantInterface {
    async fn insert_tenant(&self, tenant: domain::TenantNew) -> StoreResult<domain::Tenant>;
    async fn find_tenant_by_id(&self, tenant_id: Uuid) -> StoreResult<domain::Tenant>;
    async fn list_tenants_by_user_id(&self, user_id: Uuid) -> StoreResult<Vec<domain::Tenant>>;
}

#[async_trait::async_trait]
impl TenantInterface for Store {
    async fn insert_tenant(&self, tenant: domain::TenantNew) -> StoreResult<domain::Tenant> {
        let mut conn = self.get_conn().await?;

        let insertable_tenant: diesel_models::tenants::TenantNew = match tenant {
            TenantNew::ForOrg(tenant_new) => tenant_new.into(),
            TenantNew::ForUser(tenant_new) => {
                let org = Organization::by_user_id(&mut conn, tenant_new.user_id)
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

        diesel_models::tenants::Tenant::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_tenants_by_user_id(&self, user_id: Uuid) -> StoreResult<Vec<domain::Tenant>> {
        let mut conn = self.get_conn().await?;

        diesel_models::tenants::Tenant::list_by_user_id(&mut conn, user_id)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }
}
