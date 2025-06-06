use common_domain::ids::{ProductFamilyId, ProductId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::products::v1::{
    CreateProductRequest, CreateProductResponse, GetProductRequest, GetProductResponse,
    ListProductsRequest, ListProductsResponse, SearchProductsRequest, SearchProductsResponse,
    products_service_server::ProductsService,
};
use meteroid_store::domain;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::repositories::products::ProductInterface;
use tonic::{Request, Response, Status};

use crate::api::productitems::error::ProductApiError;
use crate::api::productitems::mapping::products::{ProductMetaWrapper, ProductWrapper};
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
                family_id: ProductFamilyId::from_proto(req.family_local_id)?,
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

        let pagination_req = req.pagination.into_domain();

        let order_by = OrderByRequest::IdAsc;

        let res = self
            .store
            .list_products(
                tenant_id,
                ProductFamilyId::from_proto_opt(req.family_local_id)?,
                pagination_req,
                order_by,
            )
            .await
            .map_err(Into::<ProductApiError>::into)?;

        let response = ListProductsResponse {
            pagination_meta: req
                .pagination
                .into_response(res.total_pages, res.total_results),
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
        let pagination_req = req.pagination.into_domain();

        let order_by = OrderByRequest::IdAsc;

        let res = self
            .store
            .search_products(
                tenant_id,
                ProductFamilyId::from_proto_opt(req.family_local_id)?,
                req.query.unwrap_or("".to_string()).as_str(), // todo add some validation on the query
                pagination_req,
                order_by,
            )
            .await
            .map_err(Into::<ProductApiError>::into)?;

        let response = SearchProductsResponse {
            pagination_meta: req
                .pagination
                .into_response(res.total_pages, res.total_results),
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

        let product_id = ProductId::from_proto(req.product_id.as_str())?;

        let res = self
            .store
            .find_product_by_id(product_id, tenant_id)
            .await
            .map_err(Into::<ProductApiError>::into)
            .map(|x| ProductWrapper::from(x).0)?;

        Ok(Response::new(GetProductResponse { product: Some(res) }))
    }
}
