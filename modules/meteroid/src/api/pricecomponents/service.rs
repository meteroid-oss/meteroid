use tonic::{Request, Response, Status};
use uuid::Uuid;

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::components::v1::{
    price_components_service_server::PriceComponentsService, CreatePriceComponentRequest,
    CreatePriceComponentResponse, EditPriceComponentRequest, EditPriceComponentResponse,
    EmptyResponse, ListPriceComponentRequest, ListPriceComponentResponse,
    RemovePriceComponentRequest,
};

use meteroid_store::repositories::price_components::PriceComponentInterface;

use crate::api::pricecomponents::error::PriceComponentApiError;
use crate::api::shared::conversions::ProtoConv;
use crate::{api::utils::parse_uuid, parse_uuid};
use common_eventbus::Event;

use super::{mapping, PriceComponentServiceComponents};

#[tonic::async_trait]
impl PriceComponentsService for PriceComponentServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_price_components(
        &self,
        request: Request<ListPriceComponentRequest>,
    ) -> Result<Response<ListPriceComponentResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let domain_components = self
            .store
            .list_price_components(parse_uuid!(&req.plan_version_id)?, tenant_id)
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
            .collect::<Result<Vec<_>, _>>()?;

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

        let mapped = mapping::components::create_api_to_domain(req.clone())?;

        let component = self
            .store
            .create_price_component(mapped)
            .await
            .map_err(|err| {
                PriceComponentApiError::StoreError(
                    "Failed to create price components".to_string(),
                    Box::new(err.into_error()),
                )
            })?;
        let response = mapping::components::domain_to_api(component.clone())?;

        let _ = self
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
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let component = mapping::components::edit_api_to_domain(req.clone())?;

        let component = self
            .store
            .update_price_component(component, tenant_id, Uuid::from_proto(req.plan_version_id)?)
            .await
            .map_err(|err| {
                PriceComponentApiError::StoreError(
                    "Failed to edit price component".to_string(),
                    Box::new(err.into_error()),
                )
            })?;
        let component = component.ok_or(Status::internal("No element was updated"))?;

        let response = mapping::components::domain_to_api(component.clone())?;

        let _ = self
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
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let price_component_id = parse_uuid(&req.price_component_id, "price_component_id")?;

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
