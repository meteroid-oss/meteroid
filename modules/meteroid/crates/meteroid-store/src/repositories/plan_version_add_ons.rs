use crate::domain::plan_version_add_ons::{PlanVersionAddOn, PlanVersionAddOnNew};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{AddOnId, PlanVersionId, TenantId};
use diesel_models::plan_version_add_ons::{PlanVersionAddOnRow, PlanVersionAddOnRowNew};
use error_stack::Report;

#[async_trait::async_trait]
pub trait PlanVersionAddOnInterface {
    async fn attach_add_on_to_plan_version(
        &self,
        new: PlanVersionAddOnNew,
    ) -> StoreResult<PlanVersionAddOn>;

    async fn detach_add_on_from_plan_version(
        &self,
        plan_version_id: PlanVersionId,
        add_on_id: AddOnId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    async fn list_plan_version_add_ons(
        &self,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PlanVersionAddOn>>;

    async fn list_plan_versions_for_add_on(
        &self,
        add_on_id: AddOnId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PlanVersionAddOn>>;
}

#[async_trait::async_trait]
impl PlanVersionAddOnInterface for Store {
    async fn attach_add_on_to_plan_version(
        &self,
        new: PlanVersionAddOnNew,
    ) -> StoreResult<PlanVersionAddOn> {
        let mut conn = self.get_conn().await?;

        let row_new: PlanVersionAddOnRowNew = new.into();

        let row = row_new
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(row.into())
    }

    async fn detach_add_on_from_plan_version(
        &self,
        plan_version_id: PlanVersionId,
        add_on_id: AddOnId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        PlanVersionAddOnRow::delete(&mut conn, plan_version_id, add_on_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn list_plan_version_add_ons(
        &self,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PlanVersionAddOn>> {
        let mut conn = self.get_conn().await?;

        let rows = PlanVersionAddOnRow::list_by_plan_version_id(
            &mut conn,
            plan_version_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_plan_versions_for_add_on(
        &self,
        add_on_id: AddOnId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PlanVersionAddOn>> {
        let mut conn = self.get_conn().await?;

        let rows = PlanVersionAddOnRow::list_by_add_on_id(&mut conn, add_on_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }
}
