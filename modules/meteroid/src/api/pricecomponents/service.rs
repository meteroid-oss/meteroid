use common_domain::ids::{BaseId, PlanVersionId, PriceComponentId, ProductId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::components::v1::{
    CreatePriceComponentRequest, CreatePriceComponentResponse, EditPriceComponentRequest,
    EditPriceComponentResponse, EmptyResponse, ListPriceComponentRequest,
    ListPriceComponentResponse, RemovePriceComponentRequest,
    price_components_service_server::PriceComponentsService,
};
use tonic::{Request, Response, Status};

use meteroid_store::domain::price_components as domain;
use meteroid_store::repositories::price_components::PriceComponentInterface;

use crate::api::pricecomponents::error::PriceComponentApiError;
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

        let components = domain_components
            .into_iter()
            .map(mapping::components::domain_to_api)
            .collect();

        let response = ListPriceComponentResponse { components };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_price_component(
        &self,
        request: Request<CreatePriceComponentRequest>,
    ) -> Result<Response<CreatePriceComponentResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;
        let product_ref = mapping::components::product_ref_from_proto(req.product)?;
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
                actor,
            )
            .await
            .map_err(|err| {
                PriceComponentApiError::StoreError(
                    "Failed to create price component".to_string(),
                    Box::new(err.into_error()),
                )
            })?;

        let response = mapping::components::domain_to_api(component.clone());

        let _ = self
            .store
            .eventbus
            .publish(Event::price_component_created(
                actor,
                component.id.as_uuid(),
                tenant_id.as_uuid(),
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
        let actor = request.actor()?;
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
            .update_price_component_with_prices(
                component,
                price_inputs,
                tenant_id,
                plan_version_id,
                actor,
            )
            .await
            .map_err(|err| {
                PriceComponentApiError::StoreError(
                    "Failed to edit price component".to_string(),
                    Box::new(err.into_error()),
                )
            })?;

        let response = mapping::components::domain_to_api(component.clone());

        let _ = self
            .store
            .eventbus
            .publish(Event::price_component_edited(
                actor,
                component.id.as_uuid(),
                tenant_id.as_uuid(),
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
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let price_component_id = PriceComponentId::from_proto(&req.price_component_id)?;

        self.store
            .delete_price_component(price_component_id, tenant_id)
            .await
            .map_err(|err| {
                PriceComponentApiError::StoreError(
                    "Failed to remove price component".to_string(),
                    Box::new(err.into_error()),
                )
            })?;

        let _ = self
            .store
            .eventbus
            .publish(Event::price_component_removed(
                actor,
                price_component_id.as_uuid(),
                tenant_id.as_uuid(),
            ))
            .await;

        Ok(Response::new(EmptyResponse {}))
    }
}
