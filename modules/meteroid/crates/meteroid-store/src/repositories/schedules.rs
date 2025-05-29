use crate::errors::StoreError;
use crate::{Store, StoreResult, domain};
use common_domain::ids::{PlanVersionId, TenantId};
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::schedules::{SchedulePatchRow, ScheduleRow, ScheduleRowNew};
use error_stack::Report;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait ScheduleInterface {
    async fn delete_schedule(&self, id: Uuid, auth_tenant_id: TenantId) -> StoreResult<()>;
    async fn list_schedules(
        &self,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<domain::Schedule>>;

    async fn insert_schedule(
        &self,
        schedule: domain::ScheduleNew,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::Schedule>;

    async fn patch_schedule(
        &self,
        schedule: domain::SchedulePatch,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::Schedule>;
}

#[async_trait::async_trait]
impl ScheduleInterface for Store {
    async fn delete_schedule(&self, id: Uuid, auth_tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        ScheduleRow::delete(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|_| ())
    }

    async fn list_schedules(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<Vec<domain::Schedule>> {
        let mut conn = self.get_conn().await?;

        ScheduleRow::list(&mut conn, plan_version_id, auth_tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn insert_schedule(
        &self,
        schedule: domain::ScheduleNew,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::Schedule> {
        let mut conn = self.get_conn().await?;

        let insertable: ScheduleRowNew = schedule.try_into()?;

        // make sure the plan version exists and belongs to auth tenant
        PlanVersionRow::find_by_id_and_tenant_id(
            &mut conn,
            insertable.plan_version_id,
            auth_tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        insertable
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)
    }

    async fn patch_schedule(
        &self,
        schedule: domain::SchedulePatch,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::Schedule> {
        let mut conn = self.get_conn().await?;

        let patch: SchedulePatchRow = schedule.try_into()?;

        patch
            .update(&mut conn, auth_tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)
    }
}
