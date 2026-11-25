use common_domain::ids::{BillableMetricId, ProductFamilyId, ProductId, TaxCategoryId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_grpc::meteroid::api::prices::v1 as prices_proto;
use meteroid_grpc::meteroid::api::products::v1::{
    ArchiveProductRequest, ArchiveProductResponse, CreateProductRequest, CreateProductResponse,
    GetProductRequest, GetProductResponse, ListProductUsageRequest, ListProductUsageResponse,
    ListProductsRequest, ListProductsResponse, ListProductsWithPricesRequest,
    ListProductsWithPricesResponse, SearchProductsRequest, SearchProductsResponse,
    UnarchiveProductRequest, UnarchiveProductResponse, UpdateProductRequest, UpdateProductResponse,
    products_service_server::ProductsService,
};
use meteroid_store::domain;
use meteroid_store::domain::prices::FeeStructure;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::prices::PriceInterface;
use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};
use tonic::{Request, Response, Status};

use crate::productitems::error::ProductApiError;
use crate::productitems::mapping::products::{
    ProductMetaWrapper, ProductWithPriceWrapper, ProductWrapper, fee_type_from_proto,
    product_list_row, product_usage_to_proto,
};
use meteroid_grpc_mapping::utils::PaginationExt;

use super::ProductServiceComponents;

/// The metric a product's charges bill, when they all bill it one way that reads one.
fn metric_of(structures: &[FeeStructure]) -> Option<BillableMetricId> {
    domain::products::single_structure(structures).and_then(FeeStructure::metric_id)
}

/// A product sent with a credit structure: a client selling credit packs by the product, behind
/// the gate of credit charges although the product keeps no structure.
fn declares_credit(fee_structure: Option<&prices_proto::FeeStructure>) -> bool {
    matches!(
        fee_structure.and_then(|fs| fs.structure.as_ref()),
        Some(prices_proto::fee_structure::Structure::Credit(_))
    )
}

#[tonic::async_trait]
impl ProductsService for ProductServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_product(
        &self,
        request: Request<CreateProductRequest>,
    ) -> Result<Response<CreateProductResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let actor_typed = request.actor_typed()?;

        // fee_type and fee_structure are not kept: the charges using the product own them. A
        // credit structure is still refused where credit pricing is not enabled.
        let req = request.into_inner();
        ensure_credit_pricing_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            declares_credit(req.fee_structure.as_ref()),
        )
        .await?;

        let res = self
            .store
            .create_product(
                actor_typed,
                domain::ProductNew {
                    name: req.name,
                    description: req.description,
                    tenant_id,
                    family_id: ProductFamilyId::from_proto(req.family_local_id)?,
                    catalog: req.catalog.unwrap_or(true),
                    tax_category_id: req
                        .tax_category_id
                        .map(TaxCategoryId::from_proto)
                        .transpose()?,
                    icon: req.icon.filter(|i| !i.is_empty()),
                    color: req.color.filter(|c| !c.is_empty()),
                },
            )
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
        let organization_id = request.organization()?;
        let actor = request.actor_typed()?;
        // fee_type and fee_structure are not kept: the charges using the product own them. A
        // credit structure is still refused where credit pricing is not enabled, unless the
        // product's charges already sell credit.
        let req = request.into_inner();

        let product_id = ProductId::from_proto(req.product_id.as_str())?;
        if declares_credit(req.fee_structure.as_ref()) {
            let sells_credit = self
                .charge_structures(tenant_id, product_id)
                .await?
                .iter()
                .any(FeeStructure::is_credit);
            ensure_credit_pricing_allowed(
                self.feature_resolver.as_ref(),
                organization_id,
                !sells_credit,
            )
            .await?;
        }

        let res = self
            .store
            .update_product(
                actor,
                ProductUpdate {
                    id: product_id,
                    tenant_id,
                    name: Some(req.name),
                    description: req.description.map(Some),
                    tax_category_id: match req.tax_category_id {
                        None => None,
                        Some(s) if s.is_empty() => Some(None),
                        Some(s) => Some(Some(TaxCategoryId::from_proto(s)?)),
                    },
                    icon: req.icon.map(|i| Some(i).filter(|i| !i.is_empty())),
                    color: req.color.map(|c| Some(c).filter(|c| !c.is_empty())),
                },
            )
            .await
            .map_err(Into::<ProductApiError>::into)?;
        let res = self.with_charges(tenant_id, res).await?;

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

        let catalog_only = req.catalog_only.unwrap_or(true);

        let res = self
            .store
            .list_products(
                tenant_id,
                ProductFamilyId::from_proto_opt(req.family_local_id)?,
                catalog_only,
                pagination_req,
                None,
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

        let catalog_only = req.catalog_only.unwrap_or(true);

        let res = self
            .store
            .search_products(
                tenant_id,
                ProductFamilyId::from_proto_opt(req.family_local_id)?,
                req.query.unwrap_or_default().as_str(), // todo add some validation on the query
                catalog_only,
                req.archived.unwrap_or(false),
                pagination_req,
                req.order_by,
            )
            .await
            .map_err(Into::<ProductApiError>::into)?;

        let ids: Vec<ProductId> = res.items.iter().map(|p| p.id).collect();
        let mut facts = self
            .store
            .product_list_facts(tenant_id, &ids)
            .await
            .map_err(Into::<ProductApiError>::into)?;

        let metric_ids: Vec<BillableMetricId> = facts
            .values()
            .filter_map(|f| metric_of(&f.charge_structures))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let metric_names: std::collections::HashMap<BillableMetricId, String> = self
            .store
            .find_billable_metrics_by_ids(&metric_ids, tenant_id)
            .await
            .map_err(Into::<ProductApiError>::into)?
            .into_iter()
            .map(|m| (m.id, m.name))
            .collect();

        let response = SearchProductsResponse {
            pagination_meta: req
                .pagination
                .into_response(res.total_pages, res.total_results),
            products: res
                .items
                .into_iter()
                .map(|product| {
                    let facts = facts.remove(&product.id);
                    let metric_name = facts
                        .as_ref()
                        .and_then(|f| metric_of(&f.charge_structures))
                        .and_then(|id| metric_names.get(&id).cloned());
                    product_list_row(product, facts, metric_name)
                })
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

        let structures = self.charge_structures(tenant_id, product_id).await?;
        let metric_name = if let Some(mid) = metric_of(&structures) {
            self.store
                .find_billable_metric_by_id(mid, tenant_id)
                .await
                .ok()
                .map(|m| m.name)
        } else {
            None
        };

        let product = ProductWrapper::with_charges(domain_product, &structures);

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

        let catalog_only = req.catalog_only.unwrap_or(true);

        let fee_types: Vec<_> = req
            .fee_types
            .iter()
            .map(|&ft| fee_type_from_proto(ft))
            .collect::<Result<_, _>>()?;

        let res = self
            .store
            .list_products_with_latest_price(
                tenant_id,
                ProductFamilyId::from_proto_opt(req.family_local_id)?,
                &req.currency,
                req.query.as_deref(),
                catalog_only,
                fee_types,
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

    #[tracing::instrument(skip_all)]
    async fn archive_product(
        &self,
        request: Request<ArchiveProductRequest>,
    ) -> Result<Response<ArchiveProductResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let product_id = ProductId::from_proto(request.into_inner().product_id.as_str())?;

        let product = self
            .store
            .archive_product(actor, product_id, tenant_id)
            .await
            .map_err(Into::<ProductApiError>::into)?;
        let product = self.with_charges(tenant_id, product).await?;

        Ok(Response::new(ArchiveProductResponse {
            product: Some(product),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn unarchive_product(
        &self,
        request: Request<UnarchiveProductRequest>,
    ) -> Result<Response<UnarchiveProductResponse>, Status> {
        let tenant_id = request.tenant()?;
        let product_id = ProductId::from_proto(request.into_inner().product_id.as_str())?;

        let product = self
            .store
            .unarchive_product(product_id, tenant_id)
            .await
            .map_err(Into::<ProductApiError>::into)?;
        let product = self.with_charges(tenant_id, product).await?;

        Ok(Response::new(UnarchiveProductResponse {
            product: Some(product),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_product_usage(
        &self,
        request: Request<ListProductUsageRequest>,
    ) -> Result<Response<ListProductUsageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let product_id = ProductId::from_proto(request.into_inner().product_id.as_str())?;

        let usage = self
            .store
            .product_usage(tenant_id, product_id)
            .await
            .map_err(Into::<ProductApiError>::into)?;

        Ok(Response::new(product_usage_to_proto(usage)))
    }
}

impl ProductServiceComponents {
    /// How the live charges using the product bill it.
    async fn charge_structures(
        &self,
        tenant_id: common_domain::ids::TenantId,
        product_id: ProductId,
    ) -> Result<Vec<FeeStructure>, Status> {
        Ok(self
            .store
            .product_list_facts(tenant_id, &[product_id])
            .await
            .map_err(Into::<ProductApiError>::into)?
            .remove(&product_id)
            .map(|facts| facts.charge_structures)
            .unwrap_or_default())
    }

    /// The product as its charges bill it.
    async fn with_charges(
        &self,
        tenant_id: common_domain::ids::TenantId,
        product: domain::Product,
    ) -> Result<meteroid_grpc::meteroid::api::products::v1::Product, Status> {
        let structures = self.charge_structures(tenant_id, product.id).await?;
        Ok(ProductWrapper::with_charges(product, &structures))
    }
}
