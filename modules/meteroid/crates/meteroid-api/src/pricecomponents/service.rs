use common_domain::ids::{BillableMetricId, PlanVersionId, PriceComponentId, ProductId, TenantId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_grpc::meteroid::api::components::v1::{
    CreatePriceComponentRequest, CreatePriceComponentResponse, EditPriceComponentRequest,
    EditPriceComponentResponse, EmptyResponse, ListPriceComponentRequest,
    ListPriceComponentResponse, RemovePriceComponentRequest,
    price_components_service_server::PriceComponentsService,
};
use std::collections::HashSet;
use tonic::{Request, Response, Status};

use meteroid_grpc::meteroid::api::billablemetrics::v1::BillableMetric as ProtoBillableMetric;
use meteroid_grpc::meteroid::api::products::v1::Product as ProtoProduct;
use meteroid_store::domain::price_components as domain;
use meteroid_store::domain::prices::FeeStructure;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;
use meteroid_store::repositories::price_components::PriceComponentInterfaceAuto;
use meteroid_store::repositories::products::ProductInterface;

use crate::billablemetrics::mapping::metric::ServerBillableMetricWrapper;
use crate::pricecomponents::error::PriceComponentApiError;
use crate::productitems::mapping::products::ProductWrapper;
use common_eventbus::Event;

use super::{PriceComponentServiceComponents, mapping};

#[tonic::async_trait]
impl PriceComponentsService for PriceComponentServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_price_components(
        &self,
        request: Request<ListPriceComponentRequest>,
    ) -> Result<Response<ListPriceComponentResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        let domain_components = self
            .store
            .list_price_components(plan_version_id, tenant_id)
            .await
            .map_err(|err| {
                PriceComponentApiError::StoreError(
                    "Failed to list price components".to_string(),
                    Box::new(err.into_error()),
                )
            })?;

        let (products, metrics) = self
            .referenced_products_and_metrics(&domain_components, tenant_id)
            .await?;

        let components = domain_components
            .into_iter()
            .map(mapping::components::domain_to_api)
            .collect();

        let response = ListPriceComponentResponse {
            components,
            products,
            metrics,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_price_component(
        &self,
        request: Request<CreatePriceComponentRequest>,
    ) -> Result<Response<CreatePriceComponentResponse>, Status> {
        let actor = request.actor_typed()?;
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;
        let product_ref = mapping::components::product_ref_from_proto(req.product)?;
        ensure_credit_pricing_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            product_ref.creates_credit_product(),
        )
        .await?;
        let price_entries = mapping::components::price_entries_from_proto(req.prices)?;

        if price_entries.is_empty() {
            return Err(Status::invalid_argument("prices must not be empty"));
        }

        let component = self
            .store
            .create_price_component_from_ref(
                req.name,
                product_ref,
                price_entries,
                plan_version_id,
                tenant_id,
            )
            .await
            .map_err(PriceComponentApiError::from)?;

        let response = mapping::components::domain_to_api(component.clone());

        let _ = self
            .store
            .eventbus
            .publish(Event::price_component_created(
                actor,
                component.id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(CreatePriceComponentResponse {
            component: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn edit_price_component(
        &self,
        request: Request<EditPriceComponentRequest>,
    ) -> Result<Response<EditPriceComponentResponse>, Status> {
        let actor = request.actor_typed()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;
        let price_entries = mapping::components::price_entries_from_proto(req.prices)?;

        // For edit, all entries must be new prices (replace all)
        let price_inputs: Vec<_> = price_entries
            .into_iter()
            .map(|entry| match entry {
                domain::PriceEntry::New(input) => Ok(input),
                domain::PriceEntry::Existing(_) => Err(Status::invalid_argument(
                    "edit_price_component does not support existing price references; send all prices as new",
                )),
            })
            .collect::<Result<_, _>>()?;

        if price_inputs.is_empty() {
            return Err(Status::invalid_argument("prices must not be empty"));
        }

        let edit_comp = req
            .component
            .ok_or(Status::invalid_argument("component is missing"))?;
        let component_id = PriceComponentId::from_proto(&edit_comp.id)?;
        let product_id = edit_comp
            .product_id
            .as_ref()
            .map(ProductId::from_proto)
            .transpose()?
            .ok_or_else(|| Status::invalid_argument("product_id is required"))?;

        let component = domain::PriceComponent {
            name: edit_comp.name,
            product_id: Some(product_id),
            id: component_id,
            prices: Vec::new(),
            legacy_pricing: None,
        };

        let component = self
            .store
            .update_price_component_with_prices(component, price_inputs, tenant_id, plan_version_id)
            .await
            .map_err(PriceComponentApiError::from)?;

        let response = mapping::components::domain_to_api(component.clone());

        let _ = self
            .store
            .eventbus
            .publish(Event::price_component_edited(
                actor,
                component.id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(EditPriceComponentResponse {
            component: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn remove_price_component(
        &self,
        request: Request<RemovePriceComponentRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let actor = request.actor_typed()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let price_component_id = PriceComponentId::from_proto(&req.price_component_id)?;

        self.store
            .delete_price_component(price_component_id, tenant_id)
            .await
            .map_err(PriceComponentApiError::from)?;

        let _ = self
            .store
            .eventbus
            .publish(Event::price_component_removed(
                actor,
                price_component_id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(EmptyResponse {}))
    }
}

impl PriceComponentServiceComponents {
    async fn referenced_products_and_metrics(
        &self,
        components: &[domain::PriceComponent],
        tenant_id: TenantId,
    ) -> Result<(Vec<ProtoProduct>, Vec<ProtoBillableMetric>), Status> {
        let product_ids: Vec<ProductId> = components
            .iter()
            .filter_map(|c| c.product_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if product_ids.is_empty() {
            return Ok((vec![], vec![]));
        }

        let store_error = |what: &str, err: error_stack::Report<_>| {
            PriceComponentApiError::StoreError(
                format!("Failed to load the components' {what}"),
                Box::new(err.into_error()),
            )
        };

        let products = self
            .store
            .find_products_by_ids(&product_ids, tenant_id)
            .await
            .map_err(|err| store_error("products", err))?;

        let metric_ids: Vec<BillableMetricId> = products
            .iter()
            .filter_map(|p| match &p.fee_structure {
                FeeStructure::Capacity { metric_id } | FeeStructure::Usage { metric_id, .. } => {
                    Some(*metric_id)
                }
                _ => None,
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let metrics = self
            .store
            .find_billable_metrics_by_ids(&metric_ids, tenant_id)
            .await
            .map_err(|err| store_error("metrics", err))?
            .into_iter()
            .map(|m| ServerBillableMetricWrapper::try_from(m).map(|w| w.0))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| store_error("metrics", err))?;

        Ok((
            products
                .into_iter()
                .map(|p| ProductWrapper::from(p).0)
                .collect(),
            metrics,
        ))
    }
}
