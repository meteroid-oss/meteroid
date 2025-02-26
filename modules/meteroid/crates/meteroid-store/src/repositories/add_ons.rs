use crate::domain::add_ons::{AddOn, AddOnNew, AddOnPatch};
use crate::domain::{PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{AddOnId, TenantId};
use diesel_models::add_ons::{AddOnRow, AddOnRowNew, AddOnRowPatch};
use error_stack::Report;

#[async_trait::async_trait]
pub trait AddOnInterface {
    async fn list_add_ons(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        search: Option<String>,
    ) -> StoreResult<PaginatedVec<AddOn>>;
    async fn get_add_on_by_id(&self, tenant_id: TenantId, id: AddOnId) -> StoreResult<AddOn>;
    async fn create_add_on(&self, add_on: AddOnNew) -> StoreResult<AddOn>;
    async fn update_add_on(&self, add_on: AddOnPatch) -> StoreResult<AddOn>;
    async fn delete_add_on(&self, id: AddOnId, tenant_id: TenantId) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl AddOnInterface for Store {
    async fn list_add_ons(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        search: Option<String>,
    ) -> StoreResult<PaginatedVec<AddOn>> {
        let mut conn = self.get_conn().await?;

        let add_ons = AddOnRow::list_by_tenant_id(&mut conn, tenant_id, pagination.into(), search)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(PaginatedVec {
            items: add_ons
                .items
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            total_pages: add_ons.total_pages,
            total_results: add_ons.total_results,
        })
    }

    async fn get_add_on_by_id(&self, tenant_id: TenantId, id: AddOnId) -> StoreResult<AddOn> {
        let mut conn = self.get_conn().await?;

        AddOnRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn create_add_on(&self, add_on: AddOnNew) -> StoreResult<AddOn> {
        let mut conn = self.get_conn().await?;

        let add_on: AddOnRowNew = add_on.try_into()?;

        add_on
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)
    }

    async fn update_add_on(&self, add_on: AddOnPatch) -> StoreResult<AddOn> {
        let mut conn = self.get_conn().await?;

        let add_on: AddOnRowPatch = add_on.try_into()?;

        add_on
            .patch(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)
    }

    async fn delete_add_on(&self, id: AddOnId, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        AddOnRow::delete(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}
