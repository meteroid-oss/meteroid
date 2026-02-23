use std::collections::HashMap;

use crate::StoreResult;
use crate::domain::enums::FeeTypeEnum;
use crate::domain::prices::FeeStructure;
use crate::domain::{
    OrderByRequest, PaginatedVec, PaginationRequest, Price, Product, ProductNew,
    ProductWithLatestPrice,
};
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::{BaseId, ProductFamilyId, ProductId, TenantId};
use diesel_models::prices::PriceRow;
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::products::{ProductRow, ProductRowNew};
use error_stack::Report;

#[derive(Clone, Debug)]
pub struct ProductUpdate {
    pub id: ProductId,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: Option<String>,
    pub fee_type: Option<FeeTypeEnum>,
    pub fee_structure: Option<FeeStructure>,
}

#[async_trait::async_trait]
pub trait ProductInterface {
    async fn create_product(&self, product: ProductNew) -> StoreResult<Product>;
    async fn update_product(&self, update: ProductUpdate) -> StoreResult<Product>;
    async fn find_product_by_id(
        &self,
        id: ProductId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<Product>;
    async fn find_products_by_ids(
        &self,
        ids: &[ProductId],
        auth_tenant_id: TenantId,
    ) -> StoreResult<Vec<Product>>;
    async fn list_products(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        catalog_only: bool,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<Product>>;
    async fn search_products(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        query: &str,
        catalog_only: bool,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<Product>>;
    async fn list_products_with_latest_price(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        currency: &str,
        query: Option<&str>,
        catalog_only: bool,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<ProductWithLatestPrice>>;
}

#[async_trait::async_trait]
impl ProductInterface for Store {
    async fn create_product(&self, product: ProductNew) -> StoreResult<Product> {
        let mut conn = self.get_conn().await?;

        let family = ProductFamilyRow::find_by_id(&mut conn, product.family_id, product.tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let insertable: ProductRowNew = ProductRowNew {
            id: ProductId::new(),
            name: product.name,
            description: product.description,
            created_by: product.created_by,
            tenant_id: product.tenant_id,
            product_family_id: family.id,
            fee_type: product.fee_type.into(),
            fee_structure: serde_json::to_value(&product.fee_structure).map_err(|e| {
                Report::new(StoreError::SerdeError(
                    "Failed to serialize fee_structure".to_string(),
                    e,
                ))
            })?,
            catalog: product.catalog,
        };

        insertable
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .and_then(|row| row.try_into())
    }

    async fn update_product(&self, update: ProductUpdate) -> StoreResult<Product> {
        let mut conn = self.get_conn().await?;

        let existing = ProductRow::find_by_id_and_tenant_id(&mut conn, update.id, update.tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // fee_type is immutable: if caller tries to change it, reject
        if let Some(new_fee_type) = update.fee_type {
            let new_db: diesel_models::enums::FeeTypeEnum = new_fee_type.into();
            if existing.fee_type != new_db {
                return Err(Report::new(StoreError::InvalidArgument(
                    "fee_type is immutable once set".to_string(),
                )));
            }
        }

        let fee_structure_json = match update.fee_structure {
            Some(fs) => serde_json::to_value(&fs).map_err(|e| {
                Report::new(StoreError::SerdeError(
                    "Failed to serialize fee_structure".to_string(),
                    e,
                ))
            })?,
            None => existing.fee_structure.clone(),
        };

        // Always pass the existing fee_type (it's NOT NULL and immutable)
        ProductRow::update_fee_structure(
            &mut conn,
            update.id,
            update.tenant_id,
            update.name,
            update.description,
            existing.fee_type,
            fee_structure_json,
        )
        .await
        .map_err(Into::into)
        .and_then(|row| row.try_into())
    }

    async fn find_product_by_id(
        &self,
        id: ProductId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<Product> {
        let mut conn = self.get_conn().await?;

        ProductRow::find_by_id_and_tenant_id(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::into)
            .and_then(|row| row.try_into())
    }

    async fn find_products_by_ids(
        &self,
        ids: &[ProductId],
        auth_tenant_id: TenantId,
    ) -> StoreResult<Vec<Product>> {
        let mut conn = self.get_conn().await?;

        let rows = ProductRow::list_by_ids(&mut conn, ids, auth_tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn list_products(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        catalog_only: bool,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<Product>> {
        let mut conn = self.get_conn().await?;

        let rows = ProductRow::list(
            &mut conn,
            auth_tenant_id,
            family_id,
            catalog_only,
            pagination.into(),
            order_by.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items: Result<Vec<Product>, _> =
            rows.items.into_iter().map(Product::try_from).collect();

        Ok(PaginatedVec {
            items: items?,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn search_products(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        query: &str,
        catalog_only: bool,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<Product>> {
        let mut conn = self.get_conn().await?;

        let rows = ProductRow::search(
            &mut conn,
            auth_tenant_id,
            family_id,
            query,
            catalog_only,
            pagination.into(),
            order_by.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items: Result<Vec<Product>, _> =
            rows.items.into_iter().map(Product::try_from).collect();

        Ok(PaginatedVec {
            items: items?,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn list_products_with_latest_price(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        currency: &str,
        query: Option<&str>,
        catalog_only: bool,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<ProductWithLatestPrice>> {
        let mut conn = self.get_conn().await?;

        // Step 1: Fetch products (search or list)
        let rows = match query {
            Some(q) if !q.is_empty() => {
                ProductRow::search(
                    &mut conn,
                    auth_tenant_id,
                    family_id,
                    q,
                    catalog_only,
                    pagination.into(),
                    OrderByRequest::NameAsc.into(),
                )
                .await
            }
            _ => {
                ProductRow::list(
                    &mut conn,
                    auth_tenant_id,
                    family_id,
                    catalog_only,
                    pagination.into(),
                    OrderByRequest::NameAsc.into(),
                )
                .await
            }
        }
        .map_err(Into::<Report<StoreError>>::into)?;

        let products: Vec<Product> = rows
            .items
            .into_iter()
            .map(Product::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        // Step 2: Batch-fetch latest price per product for the given currency
        let product_ids: Vec<ProductId> = products.iter().map(|p| p.id).collect();
        let price_rows = PriceRow::latest_by_product_ids_and_currency(
            &mut conn,
            &product_ids,
            currency,
            auth_tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let mut price_map: HashMap<ProductId, Price> = HashMap::new();
        for row in price_rows {
            let product_id = row.product_id;
            let price = Price::try_from(row)?;
            price_map.insert(product_id, price);
        }

        // Step 3: Zip products with their latest price
        let items: Vec<ProductWithLatestPrice> = products
            .into_iter()
            .map(|product| {
                let latest_price = price_map.remove(&product.id);
                ProductWithLatestPrice {
                    product,
                    latest_price,
                }
            })
            .collect();

        Ok(PaginatedVec {
            items,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }
}
