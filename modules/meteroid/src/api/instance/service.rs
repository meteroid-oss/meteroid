use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::instance::v1::instance_service_server::InstanceService;
use meteroid_grpc::meteroid::api::instance::v1::{
    GetInstanceRequest, GetInstanceResponse, GetInviteRequest, GetInviteResponse,
    InitInstanceRequest, InitInstanceResponse, Instance,
};
use meteroid_store::domain;
use meteroid_store::repositories::{OrganizationsInterface, TenantInterface};

use crate::api::instance::error::InstanceApiError;
use crate::api::instance::InstanceServiceComponents;

#[tonic::async_trait]
impl InstanceService for InstanceServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_instance(
        &self,
        _request: Request<GetInstanceRequest>,
    ) -> Result<Response<GetInstanceResponse>, Status> {


        // struct
        // - is_single_tenant
        // - is_init_instance

        // we check the is_single_tenant in config.
        // Only if true, we check if an organization exist


        // - organization opt  (if single tenant no matter connection, or if user is connected and org id is provided)

        // if single tenant


        // if the user has multiple organizations, how do we know which org to get ?
        // app.meteroid.com/ERB7C6OAI/prod/   <= in local we could avoid the /userpact , it'd be resolved to 'instance' as allow_multitenancy is false
        // userpact.meteroid.com/prod/
        // app.meteroid.com/prod/ // this requires some session and makes sharing urls/working across tabs annoying
        // localhost:3000/0/prod/  // in local multitenancy, I guess the instance can be "0"


        let maybe_instance = self
            .store
            .get_instance()
            .await
            .map_err(Into::<InstanceApiError>::into)?;


        Ok(Response::new(GetInstanceResponse {
            instance: maybe_instance.map(|org| Instance {
                company_name: org.name,
                organization_id: org.id.to_string(),
            }),
        }))
    }


    async fn get_invite(
        &self,
        request: Request<GetInviteRequest>,
    ) -> Result<Response<GetInviteResponse>, Status> {
        let tenant_id = request.tenant()?;

        let tenant = self.store.find_tenant_by_id(tenant_id).await?;

        let invite_hash = self
            .store
            .organization_get_or_create_invite_link(tenant.organization_id)
            .await
            .map_err(Into::<InstanceApiError>::into)?;

        Ok(Response::new(GetInviteResponse { invite_hash }))
    }
}
