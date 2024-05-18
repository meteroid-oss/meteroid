use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::instance::v1::instance_service_server::InstanceService;
use meteroid_grpc::meteroid::api::instance::v1::{
    GetInstanceRequest, GetInstanceResponse, GetInviteRequest, GetInviteResponse,
    InitInstanceRequest, InitInstanceResponse, Instance,
};
use meteroid_store::domain;
use meteroid_store::repositories::OrganizationsInterface;

use crate::api::instance::error::InstanceApiError;
use crate::api::instance::InstanceServiceComponents;

#[tonic::async_trait]
impl InstanceService for InstanceServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_instance(
        &self,
        _request: Request<GetInstanceRequest>,
    ) -> Result<Response<GetInstanceResponse>, Status> {
        let maybe_instance = self
            .store
            .find_organization_as_instance()
            .await
            .map_err(Into::<InstanceApiError>::into)?;

        Ok(Response::new(GetInstanceResponse {
            instance: maybe_instance.map(|org| Instance {
                company_name: org.name,
                organization_id: org.id.to_string(),
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn init_instance(
        &self,
        request: Request<InitInstanceRequest>,
    ) -> Result<Response<InitInstanceResponse>, Status> {
        let actor = request.actor()?;

        let inner = request.into_inner();

        let organization = self
            .store
            .insert_organization(
                domain::OrganizationNew {
                    name: inner.company_name,
                    slug: "instance".to_string(),
                },
                actor,
            )
            .await
            .map_err(Into::<InstanceApiError>::into)?;

        Ok(Response::new(InitInstanceResponse {
            instance: Some(Instance {
                company_name: organization.name,
                organization_id: organization.id.to_string(),
            }),
        }))
    }

    async fn get_invite(
        &self,
        _request: Request<GetInviteRequest>,
    ) -> Result<Response<GetInviteResponse>, Status> {
        let invite_hash = self
            .store
            .organization_get_or_create_invite_link()
            .await
            .map_err(Into::<InstanceApiError>::into)?;

        Ok(Response::new(GetInviteResponse { invite_hash }))
    }
}
