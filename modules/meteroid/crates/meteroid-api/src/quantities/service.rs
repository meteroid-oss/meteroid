use common_domain::ids::{FeatureId, ManagedQuantityId};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_grpc::meteroid::api::quantities::v1::{
    CreateManagedQuantityRequest, CreateManagedQuantityResponse, ListManagedQuantitiesRequest,
    ListManagedQuantitiesResponse, QuantityDecrease, QuantityIncrease,
    SetFeatureLimitQuantityRequest, SetFeatureLimitQuantityResponse, UpdateManagedQuantityRequest,
    UpdateManagedQuantityResponse, managed_quantities_service_server::ManagedQuantitiesService,
};
use meteroid_store::domain::managed_quantities::{ManagedQuantityNew, ManagedQuantityPatch};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::ManagedQuantitiesInterface;
use tonic::{Request, Response, Status};

use super::{ManagedQuantitiesServiceComponents, mapping};

#[tonic::async_trait]
impl ManagedQuantitiesService for ManagedQuantitiesServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_managed_quantities(
        &self,
        request: Request<ListManagedQuantitiesRequest>,
    ) -> Result<Response<ListManagedQuantitiesResponse>, Status> {
        let tenant = request.tenant()?;
        let quantities = self
            .store
            .list_managed_quantities(tenant)
            .await
            .map_err(store_status)?;
        Ok(Response::new(ListManagedQuantitiesResponse {
            quantities: quantities.into_iter().map(mapping::to_proto).collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_managed_quantity(
        &self,
        request: Request<CreateManagedQuantityRequest>,
    ) -> Result<Response<CreateManagedQuantityResponse>, Status> {
        let tenant = request.tenant()?;
        let inner = request.into_inner();
        let on_increase = inner
            .on_increase
            .map(|value| {
                QuantityIncrease::try_from(value)
                    .map(mapping::increase_from_proto)
                    .map_err(|_| Status::invalid_argument("unknown increase policy"))
            })
            .transpose()?
            .unwrap_or_default();
        let on_decrease = inner
            .on_decrease
            .map(|value| {
                QuantityDecrease::try_from(value)
                    .map(mapping::decrease_from_proto)
                    .map_err(|_| Status::invalid_argument("unknown decrease policy"))
            })
            .transpose()?
            .unwrap_or_default();
        let quantity = self
            .store
            .create_managed_quantity(ManagedQuantityNew {
                tenant_id: tenant,
                code: inner.code,
                name: inner.name,
                on_increase,
                on_decrease,
            })
            .await
            .map_err(store_status)?;
        Ok(Response::new(CreateManagedQuantityResponse {
            quantity: Some(mapping::to_proto(quantity)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_managed_quantity(
        &self,
        request: Request<UpdateManagedQuantityRequest>,
    ) -> Result<Response<UpdateManagedQuantityResponse>, Status> {
        let tenant = request.tenant()?;
        let inner = request.into_inner();
        let increase = inner
            .on_increase
            .map(|value| {
                QuantityIncrease::try_from(value)
                    .map(mapping::increase_from_proto)
                    .map_err(|_| Status::invalid_argument("unknown increase policy"))
            })
            .transpose()?;
        let decrease = inner
            .on_decrease
            .map(|value| {
                QuantityDecrease::try_from(value)
                    .map(mapping::decrease_from_proto)
                    .map_err(|_| Status::invalid_argument("unknown decrease policy"))
            })
            .transpose()?;
        let quantity = self
            .store
            .update_managed_quantity(ManagedQuantityPatch {
                tenant_id: tenant,
                id: ManagedQuantityId::from_proto(inner.id)?,
                name: inner.name,
                on_increase: increase,
                on_decrease: decrease,
            })
            .await
            .map_err(store_status)?;
        Ok(Response::new(UpdateManagedQuantityResponse {
            quantity: Some(mapping::to_proto(quantity)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_feature_limit_quantity(
        &self,
        request: Request<SetFeatureLimitQuantityRequest>,
    ) -> Result<Response<SetFeatureLimitQuantityResponse>, Status> {
        let tenant = request.tenant()?;
        let inner = request.into_inner();
        let quantity = inner
            .managed_quantity_id
            .map(ManagedQuantityId::from_proto)
            .transpose()?;
        self.store
            .set_feature_limit_quantity(tenant, FeatureId::from_proto(inner.feature_id)?, quantity)
            .await
            .map_err(store_status)?;
        Ok(Response::new(SetFeatureLimitQuantityResponse {}))
    }
}

fn store_status(report: Report<StoreError>) -> Status {
    match report.current_context() {
        StoreError::InvalidArgument(message) => Status::invalid_argument(message.clone()),
        StoreError::ValueNotFound(message) => Status::not_found(message.clone()),
        StoreError::DuplicateValue { .. } => {
            Status::already_exists("A quantity of this code exists already")
        }
        _ => Status::internal("Error in managed quantities service"),
    }
}
