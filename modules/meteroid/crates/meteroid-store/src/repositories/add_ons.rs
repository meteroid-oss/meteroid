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
    async fn list_add_ons_by_ids(
        &self,
        tenant_id: TenantId,
        ids: Vec<AddOnId>,
    ) -> StoreResult<Vec<AddOn>>;
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
            items: add_ons.items.into_iter().map(Into::into).collect(),
            total_pages: add_ons.total_pages,
            total_results: add_ons.total_results,
        })
    }

    async fn list_add_ons_by_ids(
        &self,
        tenant_id: TenantId,
        ids: Vec<AddOnId>,
    ) -> StoreResult<Vec<AddOn>> {
        let mut conn = self.get_conn().await?;

        AddOnRow::list_by_ids(&mut conn, &ids, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }

    async fn get_add_on_by_id(&self, tenant_id: TenantId, id: AddOnId) -> StoreResult<AddOn> {
        let mut conn = self.get_conn().await?;

        AddOnRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn create_add_on(&self, add_on: AddOnNew) -> StoreResult<AddOn> {
        let mut conn = self.get_conn().await?;

        // Validate price belongs to product if both are provided
        if let (Some(product_id), Some(price_id)) = (add_on.product_id, add_on.price_id) {
            let price_row =
                diesel_models::prices::PriceRow::find_by_id_and_tenant_id(
                    &mut conn,
                    price_id,
                    add_on.tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            if price_row.product_id != product_id {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "Price {} belongs to product {}, not {}",
                    price_id, price_row.product_id, product_id
                ))));
            }
        }

        let add_on: AddOnRowNew = add_on.into();

        add_on
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)
    }

    async fn update_add_on(&self, add_on: AddOnPatch) -> StoreResult<AddOn> {
        let mut conn = self.get_conn().await?;

        // If updating price_id, validate it belongs to the product
        if let Some(Some(price_id)) = add_on.price_id {
            // Get the effective product_id (from patch if changing, else from existing)
            let effective_product_id = if let Some(Some(pid)) = add_on.product_id {
                Some(pid)
            } else {
                let existing =
                    AddOnRow::get_by_id(&mut conn, add_on.tenant_id, add_on.id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                existing.product_id
            };

            if let Some(product_id) = effective_product_id {
                let price_row =
                    diesel_models::prices::PriceRow::find_by_id_and_tenant_id(
                        &mut conn,
                        price_id,
                        add_on.tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if price_row.product_id != product_id {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Price {} belongs to product {}, not {}",
                        price_id, price_row.product_id, product_id
                    ))));
                }
            }
        }

        let add_on: AddOnRowPatch = add_on.into();

        add_on
            .patch(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)
    }

    async fn delete_add_on(&self, id: AddOnId, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        AddOnRow::delete(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}
