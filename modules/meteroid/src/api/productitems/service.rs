use common_domain::ids::{BillableMetricId, ProductFamilyId, ProductId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::products::v1::{
    CreateProductRequest, CreateProductResponse, GetProductRequest, GetProductResponse,
    ListProductsRequest, ListProductsResponse, ListProductsWithPricesRequest,
    ListProductsWithPricesResponse, SearchProductsRequest, SearchProductsResponse,
    UpdateProductRequest, UpdateProductResponse, products_service_server::ProductsService,
};
use meteroid_store::domain;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::domain::prices::FeeStructure;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::prices::PriceInterface;
use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};
use tonic::{Request, Response, Status};

use crate::api::productitems::error::ProductApiError;
use crate::api::productitems::mapping::products::{
    ProductMetaWrapper, ProductWithPriceWrapper, ProductWrapper, fee_structure_from_proto,
    fee_type_from_proto,
};
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

        let fee_type = fee_type_from_proto(req.fee_type)?;
        let fee_structure = req
            .fee_structure
            .map(fee_structure_from_proto)
            .transpose()?
            .ok_or_else(|| Status::invalid_argument("fee_structure is required"))?;

        let res = self
            .store
            .create_product(domain::ProductNew {
                name: req.name,
                description: req.description,
                created_by: actor,
                tenant_id,
                family_id: ProductFamilyId::from_proto(req.family_local_id)?,
                fee_type,
                fee_structure,
            })
            .await
            .map_err(Into::<ProductApiError>::into)
            .map(|x| ProductWrapper::from(x).0)?;

        Ok(Response::new(CreateProductResponse { product: Some(res) }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_product(
        &self,
        request: Request<UpdateProductRequest>,
    ) -> Result<Response<UpdateProductResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let product_id = ProductId::from_proto(req.product_id.as_str())?;
        let fee_type = req.fee_type.map(fee_type_from_proto).transpose()?;
        let fee_structure = req
            .fee_structure
            .map(fee_structure_from_proto)
            .transpose()?;

        let res = self
            .store
            .update_product(ProductUpdate {
                id: product_id,
                tenant_id,
                name: req.name,
                description: req.description,
                fee_type,
                fee_structure,
            })
            .await
            .map_err(Into::<ProductApiError>::into)
            .map(|x| ProductWrapper::from(x).0)?;

        Ok(Response::new(UpdateProductResponse { product: Some(res) }))
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
                req.query.unwrap_or_default().as_str(), // todo add some validation on the query
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

        let domain_product = self
            .store
            .find_product_by_id(product_id, tenant_id)
            .await
            .map_err(Into::<ProductApiError>::into)?;

        let metric_id: Option<BillableMetricId> = match &domain_product.fee_structure {
            FeeStructure::Capacity { metric_id } | FeeStructure::Usage { metric_id, .. } => {
                Some(*metric_id)
            }
            _ => None,
        };

        let metric_name = if let Some(mid) = metric_id {
            self.store
                .find_billable_metric_by_id(mid, tenant_id)
                .await
                .ok()
                .map(|m| m.name)
        } else {
            None
        };

        let product = ProductWrapper::from(domain_product).0;

        let prices = self
            .store
            .list_prices_by_product_id(product_id, tenant_id)
            .await
            .map_err(Into::<ProductApiError>::into)?;

        let mut currencies: Vec<String> = prices
            .iter()
            .map(|p| p.currency.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        currencies.sort();

        Ok(Response::new(GetProductResponse {
            product: Some(product),
            currencies,
            metric_name,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_products_with_prices(
        &self,
        request: Request<ListProductsWithPricesRequest>,
    ) -> Result<Response<ListProductsWithPricesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let pagination_req = req.pagination.into_domain();

        let res = self
            .store
            .list_products_with_latest_price(
                tenant_id,
                ProductFamilyId::from_proto_opt(req.family_local_id)?,
                &req.currency,
                req.query.as_deref(),
                pagination_req,
            )
            .await
            .map_err(Into::<ProductApiError>::into)?;

        let response = ListProductsWithPricesResponse {
            pagination_meta: req
                .pagination
                .into_response(res.total_pages, res.total_results),
            products: res
                .items
                .into_iter()
                .map(|x| ProductWithPriceWrapper::from(x).0)
                .collect(),
        };

        Ok(Response::new(response))
    }
}
