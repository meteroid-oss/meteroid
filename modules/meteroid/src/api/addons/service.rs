use crate::api::addons::AddOnsServiceComponents;
use crate::api::addons::error::AddOnApiError;
use crate::api::addons::mapping::addons::AddOnWrapper;
use crate::api::utils::PaginationExt;
use common_domain::ids::{AddOnId, PriceId, ProductId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::addons::v1::add_ons_service_server::AddOnsService;
use meteroid_grpc::meteroid::api::addons::v1::{
    CreateAddOnRequest, CreateAddOnResponse, EditAddOnRequest, EditAddOnResponse, ListAddOnRequest,
    ListAddOnResponse, RemoveAddOnRequest, RemoveAddOnResponse,
};
use meteroid_store::domain::add_ons::{AddOnNew, AddOnPatch};
use meteroid_store::repositories::add_ons::AddOnInterface;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl AddOnsService for AddOnsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_add_ons(
        &self,
        request: Request<ListAddOnRequest>,
    ) -> Result<Response<ListAddOnResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let pagination_req = req.pagination.into_domain();

        let add_ons = self
            .store
            .list_add_ons(tenant_id, pagination_req, req.search)
            .await
            .map_err(Into::<AddOnApiError>::into)?;

        let response = ListAddOnResponse {
            pagination_meta: req
                .pagination
                .into_response(add_ons.total_pages, add_ons.total_results),
            add_ons: add_ons
                .items
                .into_iter()
                .map(|x| AddOnWrapper::from(x).0)
                .collect(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_add_on(
        &self,
        request: Request<CreateAddOnRequest>,
    ) -> Result<Response<CreateAddOnResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let product_id = req
            .product_id
            .map(|id| ProductId::from_proto(&id))
            .transpose()?;
        let price_id = req
            .price_id
            .map(|id| PriceId::from_proto(&id))
            .transpose()?;

        let new = AddOnNew {
            tenant_id,
            name: req.name,
            plan_version_id: None,
            product_id,
            price_id,
        };
        let added = self
            .store
            .create_add_on(new)
            .await
            .map(|x| AddOnWrapper::from(x).0)
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(CreateAddOnResponse {
            add_on: Some(added),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn remove_add_on(
        &self,
        request: Request<RemoveAddOnRequest>,
    ) -> Result<Response<RemoveAddOnResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let add_on_id = AddOnId::from_proto(&req.add_on_id)?;

        self.store
            .delete_add_on(add_on_id, tenant_id)
            .await
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(RemoveAddOnResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn edit_add_on(
        &self,
        request: Request<EditAddOnRequest>,
    ) -> Result<Response<EditAddOnResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let add_on = req
            .add_on
            .ok_or(AddOnApiError::MissingArgument("add_on".into()))?;

        let product_id = add_on
            .product_id
            .map(|id| ProductId::from_proto(&id))
            .transpose()?;
        let price_id = add_on
            .price_id
            .map(|id| PriceId::from_proto(&id))
            .transpose()?;

        let patch = AddOnPatch {
            id: AddOnId::from_proto(&add_on.id)?,
            tenant_id,
            name: Some(add_on.name),
            plan_version_id: None,
            product_id: product_id.map(Some),
            price_id: price_id.map(Some),
        };

        let edited = self
            .store
            .update_add_on(patch)
            .await
            .map(|x| AddOnWrapper::from(x).0)
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(EditAddOnResponse {
            add_on: Some(edited),
        }))
    }
}
