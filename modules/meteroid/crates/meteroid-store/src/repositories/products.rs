use crate::domain::{OrderByRequest, PaginatedVec, PaginationRequest, Product, ProductNew};
use crate::errors::StoreError;
use crate::store::Store;
use crate::utils::local_id::{IdType, LocalId};
use crate::StoreResult;
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::products::{ProductRow, ProductRowNew};
use error_stack::Report;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait ProductInterface {
    async fn create_product(&self, product: ProductNew) -> StoreResult<Product>;
    async fn find_product_by_id(&self, id: Uuid, auth_tenant_id: Uuid) -> StoreResult<Product>;
    async fn list_products(
        &self,
        auth_tenant_id: Uuid,
        family_local_id: &str,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<Product>>;
    async fn search_products(
        &self,
        auth_tenant_id: Uuid,
        family_local_id: &str,
        query: &str,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<Product>>;
}

#[async_trait::async_trait]
impl ProductInterface for Store {
    async fn create_product(&self, product: ProductNew) -> StoreResult<Product> {
        let mut conn = self.get_conn().await?;

        let family = ProductFamilyRow::find_by_local_id_and_tenant_id(
            &mut conn,
            product.family_local_id.as_str(),
            product.tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let insertable = ProductRowNew {
            id: Uuid::now_v7(),
            local_id: LocalId::generate_for(IdType::Product),
            name: product.name,
            description: product.description,
            created_by: product.created_by,
            tenant_id: product.tenant_id,
            product_family_id: family.id,
        };

        insertable
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_product_by_id(&self, id: Uuid, auth_tenant_id: Uuid) -> StoreResult<Product> {
        let mut conn = self.get_conn().await?;

        ProductRow::find_by_id_and_tenant_id(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_products(
        &self,
        auth_tenant_id: Uuid,
        family_local_id: &str,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<Product>> {
        let mut conn = self.get_conn().await?;

        let rows = ProductRow::list(
            &mut conn,
            auth_tenant_id,
            family_local_id,
            pagination.into(),
            order_by.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<Product> = PaginatedVec {
            items: rows.items.into_iter().map(|s| s.into()).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn search_products(
        &self,
        auth_tenant_id: Uuid,
        family_local_id: &str,
        query: &str,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<Product>> {
        let mut conn = self.get_conn().await?;

        let rows = ProductRow::search(
            &mut conn,
            auth_tenant_id,
            family_local_id,
            query,
            pagination.into(),
            order_by.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<Product> = PaginatedVec {
            items: rows.items.into_iter().map(|s| s.into()).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }
}
