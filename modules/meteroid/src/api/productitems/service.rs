use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::products::v1::{
    products_service_server::ProductsService, CreateProductRequest, CreateProductResponse,
    GetProductRequest, GetProductResponse, ListProductsRequest, ListProductsResponse,
    SearchProductsRequest, SearchProductsResponse,
};
use meteroid_store::domain;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::repositories::products::ProductInterface;

use crate::api::productitems::error::ProductApiError;
use crate::api::productitems::mapping::products::{ProductMetaWrapper, ProductWrapper};
use crate::api::utils::parse_uuid;
use crate::api::utils::PaginationExt;

use super::ProductServiceComponents;

#[tonic::async_trait]
impl ProductsService for ProductServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_product(
        &self,
        request: Request<CreateProductRequest>,
    ) -> Result<Response<CreateProductResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;

        let req = request.into_inner();

        let res = self
            .store
            .create_product(domain::ProductNew {
                name: req.name,
                description: req.description,
                created_by: actor,
                tenant_id,
                family_local_id: req.family_local_id,
            })
            .await
            .map_err(Into::<ProductApiError>::into)
            .map(|x| ProductWrapper::from(x).0)?;

        Ok(Response::new(CreateProductResponse { product: Some(res) }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_products(
        &self,
        request: Request<ListProductsRequest>,
    ) -> Result<Response<ListProductsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let pagination_req = domain::PaginationRequest {
            page: req.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: req.pagination.as_ref().map(|p| p.limit),
        };

        let order_by = OrderByRequest::IdAsc;

        let res = self
            .store
            .list_products(
                tenant_id,
                req.family_local_id.as_str(),
                pagination_req,
                order_by,
            )
            .await
            .map_err(Into::<ProductApiError>::into)?;

        let response = ListProductsResponse {
            pagination_meta: req.pagination.into_response(res.total_results as u32),
            products: res
                .items
                .into_iter()
                .map(|x| ProductMetaWrapper::from(x).0)
                .collect::<Vec<_>>(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn search_products(
        &self,
        request: Request<SearchProductsRequest>,
    ) -> Result<Response<SearchProductsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let pagination_req = domain::PaginationRequest {
            page: req.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: req.pagination.as_ref().map(|p| p.limit),
        };

        let order_by = OrderByRequest::IdAsc;

        let res = self
            .store
            .search_products(
                tenant_id,
                req.family_local_id.as_str(),
                req.query.unwrap_or("".to_string()).as_str(), // todo add some validation on the query
                pagination_req,
                order_by,
            )
            .await
            .map_err(Into::<ProductApiError>::into)?;

        let response = SearchProductsResponse {
            pagination_meta: req.pagination.into_response(res.total_results as u32),
            products: res
                .items
                .into_iter()
                .map(|x| ProductMetaWrapper::from(x).0)
                .collect::<Vec<_>>(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_product(
        &self,
        request: Request<GetProductRequest>,
    ) -> Result<Response<GetProductResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let product_id = parse_uuid(req.product_id.as_str(), "product_id")?;

        let res = self
            .store
            .find_product_by_id(product_id, tenant_id)
            .await
            .map_err(Into::<ProductApiError>::into)
            .map(|x| ProductWrapper::from(x).0)?;

        Ok(Response::new(GetProductResponse { product: Some(res) }))
    }
}
