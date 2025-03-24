use crate::presets;
use common_domain::ids::OrganizationId;
use meteroid_store::domain::Tenant;
use meteroid_store::{Store, StoreResult};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait SeederInterface {
    async fn insert_seeded_sandbox_tenant(
        &self,
        tenant_name: String,
        organization_id: OrganizationId,
        user_id: Uuid,
    ) -> StoreResult<Tenant>;
}

#[async_trait::async_trait]
impl SeederInterface for Store {
    async fn insert_seeded_sandbox_tenant(
        &self,
        tenant_name: String,
        organization_id: OrganizationId,
        user_id: Uuid,
    ) -> StoreResult<Tenant> {
        let tenant = presets::run_preset(
            self,
            presets::simple::basic_scenario_1(),
            organization_id,
            user_id,
            Some(tenant_name),
        )
        .await?;

        Ok(tenant)
    }
}
