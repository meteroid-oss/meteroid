use crate::api::addons::error::AddOnApiError;
use crate::api::addons::mapping::addons::AddOnWrapper;
use crate::api::addons::AddOnsServiceComponents;
use crate::{api::utils::parse_uuid, parse_uuid};
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

        let add_ons = self
            .store
            .list_add_ons(tenant_id)
            .await
            .map_err(Into::<AddOnApiError>::into)?
            .into_iter()
            .map(|x| AddOnWrapper::from(x).0)
            .collect();

        let response = ListAddOnResponse { add_ons };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_add_on(
        &self,
        request: Request<CreateAddOnRequest>,
    ) -> Result<Response<CreateAddOnResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let fee = crate::api::pricecomponents::mapping::components::map_fee_to_domain(req.fee)?;

        let new = AddOnNew {
            tenant_id,
            name: req.name,
            fee,
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

        let add_on_id = parse_uuid!(&req.add_on_id)?;

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

        let fee = crate::api::pricecomponents::mapping::components::map_fee_to_domain(add_on.fee)?;

        let patch = AddOnPatch {
            id: parse_uuid!(&add_on.id)?,
            tenant_id,
            name: Some(add_on.name),
            fee: Some(fee),
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
