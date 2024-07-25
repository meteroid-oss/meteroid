use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::tenants::v1::{
    tenants_service_server::TenantsService, ActiveTenantRequest, ActiveTenantResponse,
    ConfigureTenantBillingRequest, ConfigureTenantBillingResponse, CreateTenantRequest,
    CreateTenantResponse, GetTenantByIdRequest, GetTenantByIdResponse, ListTenantsRequest,
    ListTenantsResponse,
};
use meteroid_store::domain;
use meteroid_store::repositories::configs::ConfigsInterface;
use meteroid_store::repositories::{ProductFamilyInterface, TenantInterface};

use crate::api::tenants::error::TenantApiError;
use crate::{api::utils::parse_uuid, parse_uuid};

use super::{mapping, TenantServiceComponents};

#[tonic::async_trait]
impl TenantsService for TenantServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn active_tenant(
        &self,
        request: Request<ActiveTenantRequest>,
    ) -> Result<Response<ActiveTenantResponse>, Status> {
        let tenant_id = request.tenant()?;

        let tenant = self
            .store
            .find_tenant_by_id(tenant_id)
            .await
            .map(mapping::tenants::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(ActiveTenantResponse {
            tenant: Some(tenant),
            billing_config: None, // todo load it from provider_config if needed
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_tenants(
        &self,
        request: Request<ListTenantsRequest>,
    ) -> Result<Response<ListTenantsResponse>, Status> {
        let result = self
            .store
            .list_tenants_by_user_id(request.actor()?)
            .await
            .map(|x| {
                x.into_iter()
                    .map(mapping::tenants::domain_to_server)
                    .collect()
            })
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(ListTenantsResponse { tenants: result }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_tenant_by_id(
        &self,
        request: Request<GetTenantByIdRequest>,
    ) -> Result<Response<GetTenantByIdResponse>, Status> {
        let req = request.into_inner();
        let tenant_id = parse_uuid!(&req.tenant_id)?;

        let tenant = self
            .store
            .find_tenant_by_id(tenant_id)
            .await
            .map(mapping::tenants::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(GetTenantByIdResponse {
            tenant: Some(tenant),
            billing_config: None, // todo load it from provider_config if needed
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_tenant(
        &self,
        request: Request<CreateTenantRequest>,
    ) -> Result<Response<CreateTenantResponse>, Status> {
        let actor = request.actor()?;

        let req = mapping::tenants::create_req_to_domain(request.into_inner(), actor);

        let res = self
            .store
            .insert_tenant(req)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        self.store
            .insert_product_family(
                domain::ProductFamilyNew {
                    name: "Default".to_string(),
                    external_id: "default".to_string(),
                    tenant_id: res.id.clone(),
                },
                Some(actor),
            )
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(CreateTenantResponse {
            tenant: Some(mapping::tenants::domain_to_server(res)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn configure_tenant_billing(
        &self,
        request: Request<ConfigureTenantBillingRequest>,
    ) -> Result<Response<ConfigureTenantBillingResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let cfg = mapping::provider_configs::create_req_server_to_domain(req, tenant_id)?;

        let res = self
            .store
            .insert_provider_config(cfg)
            .await
            .map(mapping::provider_configs::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(ConfigureTenantBillingResponse {
            billing_config: Some(res),
        }))
    }
}
