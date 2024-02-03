use common_grpc::middleware::server::auth::RequestExt;
use db::products::ListProductsParams;
use meteroid_grpc::meteroid::api::products::v1::{
    products_service_server::ProductsService, CreateProductRequest, CreateProductResponse,
    GetProductRequest, GetProductResponse, ListProductsRequest, ListProductsResponse,
    SearchProductsRequest, SearchProductsResponse,
};
use meteroid_repository as db;
use std::sync::Arc;
use tonic::{Request, Response, Status};

use super::mapping;
use crate::api::services::utils::PaginationExt;
use crate::{
    api::services::utils::{parse_uuid, uuid_gen},
    db::DbService,
    parse_uuid,
};
use meteroid_repository::products::SearchProductsParams;
use meteroid_repository::Params;

#[tonic::async_trait]
impl ProductsService for DbService {
    #[tracing::instrument(skip_all)]
    async fn create_product(
        &self,
        request: Request<CreateProductRequest>,
    ) -> Result<Response<CreateProductResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;

        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let product_id = uuid_gen::v7();

        let params = db::products::UpsertProductParams {
            id: product_id,
            name: req.name,
            description: req.description,
            tenant_id,
            created_by: actor,
            product_family_external_id: req.family_external_id,
        };

        let new_product = db::products::upsert_product()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to create product")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let res = mapping::products::db_to_server(new_product);
        Ok(Response::new(CreateProductResponse { product: Some(res) }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_products(
        &self,
        request: Request<ListProductsRequest>,
    ) -> Result<Response<ListProductsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let params = ListProductsParams {
            tenant_id,
            family_external_id: req.family_external_id,
            offset: req.pagination.offset(),
            limit: req.pagination.limit(),
        };

        let products = db::products::list_products()
            .params(&connection, &params)
            .all()
            .await
            .map_err(|e| {
                Status::internal("Unable to list products")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let total_count = products.first().map(|p| p.total_count).unwrap_or(0);

        let res = ListProductsResponse {
            products: products
                .into_iter()
                .map(|f| mapping::products::db_to_server_list(f))
                .collect::<Vec<_>>(),
            pagination_meta: req.pagination.into_response(total_count as u32),
        };

        Ok(Response::new(res))
    }

    #[tracing::instrument(skip_all)]
    async fn search_products(
        &self,
        request: Request<SearchProductsRequest>,
    ) -> Result<Response<SearchProductsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let params = SearchProductsParams {
            tenant_id,
            family_external_id: req.family_external_id,
            offset: req.pagination.offset(),
            limit: req.pagination.limit(),
            query: req.query,
        };

        let products = db::products::search_products()
            .params(&connection, &params)
            .all()
            .await
            .map_err(|e| {
                Status::internal("Unable to search products")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let total_count = products.first().map(|p| p.total_count).unwrap_or(0);

        let res = SearchProductsResponse {
            products: products
                .into_iter()
                .map(mapping::products::db_to_server_list)
                .collect(),
            pagination_meta: req.pagination.into_response(total_count as u32),
        };

        Ok(Response::new(res))
    }

    #[tracing::instrument(skip_all)]
    async fn get_product(
        &self,
        request: Request<GetProductRequest>,
    ) -> Result<Response<GetProductResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let params = db::products::GetProductDetailsParams {
            product_id: parse_uuid!(&req.product_id)?,
            tenant_id,
        };

        let product = db::products::get_product_details()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to get product")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let res = mapping::products::db_to_server(product);
        Ok(Response::new(GetProductResponse { product: Some(res) }))
    }
}
