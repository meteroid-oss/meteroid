use common_domain::ids::{
    BillableMetricId, PlanOptionId, PlanVersionId, PriceComponentId, ProductId, TenantId,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_grpc::meteroid::api::components::v1::{
    ClearComponentPriceRequest, ClearComponentPriceResponse, CreatePriceComponentRequest,
    CreatePriceComponentResponse, EditPriceComponentRequest, EditPriceComponentResponse,
    EmptyResponse, ListPriceComponentRequest, ListPriceComponentResponse,
    RemovePriceComponentRequest, SetComponentPriceRequest, SetComponentPriceResponse,
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
        // The charge's shape; a new product's is taken for clients that send it there.
        let fee_structure = mapping::components::charge_structure_from_proto(req.fee_structure)?;
        let product_ref = self
            .charge_product_ref(
                tenant_id,
                actor.clone(),
                plan_version_id,
                req.product,
                fee_structure.as_ref(),
                req.prices.is_empty(),
            )
            .await?;
        ensure_credit_pricing_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            fee_structure
                .as_ref()
                .or_else(|| product_ref.new_product_structure())
                .is_some_and(FeeStructure::is_credit),
        )
        .await?;
        let price_entries = mapping::components::price_entries_from_proto(req.prices)?;

        // No price includes the product, its features only (see `charge_product_ref`).
        let component = self
            .store
            .create_charge_from_ref(
                req.name,
                product_ref,
                fee_structure,
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
        let organization_id = request.organization()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;
        let price_entries = mapping::components::price_entries_from_proto(req.prices)?;

        // New prices replace the component's others, an existing one is kept as it is.
        let mut price_inputs = Vec::new();
        let mut keep = Vec::new();
        for entry in price_entries {
            match entry {
                domain::PriceEntry::New(input) => price_inputs.push(input),
                domain::PriceEntry::Existing(id) => keep.push(id),
            }
        }
        // No price, none kept: the charge becomes included.
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

        // A new shape when the request sets one (drafts only), else the charge keeps its own.
        let stored = self
            .store
            .get_price_component_by_id(tenant_id, component_id)
            .await
            .map_err(PriceComponentApiError::from)?
            .fee_structure;
        let fee_structure =
            match mapping::components::charge_structure_from_proto(req.fee_structure)? {
                Some(structure) => {
                    // Turning a charge into a credit pack introduces credit pricing.
                    ensure_credit_pricing_allowed(
                        self.feature_resolver.as_ref(),
                        organization_id,
                        structure.is_credit()
                            && !stored.as_ref().is_some_and(FeeStructure::is_credit),
                    )
                    .await?;
                    Some(structure)
                }
                None => stored,
            };

        let component = domain::PriceComponent {
            name: edit_comp.name,
            product_id: Some(product_id),
            fee_structure,
            id: component_id,
            prices: Vec::new(),
            option_prices: Vec::new(),
            legacy_pricing: None,
        };

        let component = self
            .store
            .update_price_component_with_prices(
                component,
                price_inputs,
                keep,
                tenant_id,
                plan_version_id,
            )
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

    #[tracing::instrument(skip_all)]
    async fn set_component_price(
        &self,
        request: Request<SetComponentPriceRequest>,
    ) -> Result<Response<SetComponentPriceResponse>, Status> {
        let actor = request.actor_typed()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let component_id = PriceComponentId::from_proto(&req.price_component_id)?;
        let options = option_ids_from_proto(&req.option_ids)?;

        // `prices` is the whole cell (a capacity charge's levels); `price` alone, one price.
        let component = if req.prices.is_empty() {
            let price = req
                .price
                .ok_or_else(|| Status::invalid_argument("price is required"))?;
            let price = mapping::components::price_input_from_proto(price)?;
            self.store
                .set_component_price(tenant_id, component_id, options, price)
                .await
        } else {
            let prices = req
                .prices
                .into_iter()
                .map(mapping::components::price_input_from_proto)
                .collect::<Result<Vec<_>, _>>()?;
            self.store
                .set_component_prices(tenant_id, component_id, options, prices)
                .await
        }
        .map_err(PriceComponentApiError::from)?;

        let _ = self
            .store
            .eventbus
            .publish(Event::price_component_edited(
                actor,
                component.id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(SetComponentPriceResponse {
            component: Some(mapping::components::domain_to_api(component)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn clear_component_price(
        &self,
        request: Request<ClearComponentPriceRequest>,
    ) -> Result<Response<ClearComponentPriceResponse>, Status> {
        let actor = request.actor_typed()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let component_id = PriceComponentId::from_proto(&req.price_component_id)?;
        let currency = req.currency.trim().to_ascii_uppercase();
        if currency.is_empty() {
            return Err(Status::invalid_argument("currency is required"));
        }
        let options = option_ids_from_proto(&req.option_ids)?;

        let component = self
            .store
            .clear_component_price(tenant_id, component_id, options, currency)
            .await
            .map_err(PriceComponentApiError::from)?;

        let _ = self
            .store
            .eventbus
            .publish(Event::price_component_edited(
                actor,
                component.id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(ClearComponentPriceResponse {
            component: Some(mapping::components::domain_to_api(component)),
        }))
    }
}

fn option_ids_from_proto(ids: &[String]) -> Result<Vec<PlanOptionId>, Status> {
    ids.iter().map(PlanOptionId::from_proto).collect()
}

impl PriceComponentServiceComponents {
    /// The product a new charge bills. A new product with no structure, neither its own nor
    /// the charge's, is a product included with no price: created first, as it holds none.
    async fn charge_product_ref(
        &self,
        tenant_id: TenantId,
        actor: common_domain::actor::Actor,
        plan_version_id: PlanVersionId,
        product: Option<meteroid_grpc::meteroid::api::components::v1::ProductRef>,
        fee_structure: Option<&FeeStructure>,
        no_price: bool,
    ) -> Result<domain::ProductRef, Status> {
        use meteroid_grpc::meteroid::api::components::v1::product_ref::Ref;
        use meteroid_store::repositories::plans::PlansInterfaceAuto;

        let included = match product.as_ref().and_then(|p| p.r#ref.as_ref()) {
            Some(Ref::NewProduct(np)) if np.fee_structure.is_none() && fee_structure.is_none() => {
                Some(np.name.clone())
            }
            _ => None,
        };
        let Some(name) = included else {
            return mapping::components::product_ref_from_proto(product, fee_structure);
        };
        if !no_price {
            return Err(Status::invalid_argument(
                "a priced charge names how it bills its product",
            ));
        }
        let plan = self
            .store
            .get_plan_by_version_id(plan_version_id, tenant_id)
            .await
            .map_err(PriceComponentApiError::from)?;
        let product = self
            .store
            .create_product(
                actor,
                meteroid_store::domain::ProductNew {
                    name,
                    description: None,
                    tenant_id,
                    family_id: plan.plan.product_family_id,
                    catalog: true,
                    tax_category_id: None,
                    icon: None,
                    color: None,
                },
            )
            .await
            .map_err(PriceComponentApiError::from)?;
        Ok(domain::ProductRef::Existing(product.id))
    }

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

        // The metrics the charges bill, each read from its own structure.
        let metric_ids: Vec<BillableMetricId> = components
            .iter()
            .filter_map(|c| c.structure().and_then(FeeStructure::metric_id))
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
