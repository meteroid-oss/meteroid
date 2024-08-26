use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::tenants::v1::{tenants_service_server::TenantsService, ActiveTenantRequest, ActiveTenantResponse, ConfigureTenantBillingRequest, ConfigureTenantBillingResponse, CreateTenantRequest, CreateTenantResponse, GetTenantByIdRequest, GetTenantByIdResponse, ListTenantsRequest, ListTenantsResponse, UpdateTenantRequest, UpdateTenantResponse};
use meteroid_middleware::server::auth::strategies::jwt_strategy::invalidate_resolve_slugs_cache;
use meteroid_store::repositories::configs::ConfigsInterface;
use meteroid_store::repositories::{OrganizationsInterface, TenantInterface};

use crate::api::tenants::error::TenantApiError;
use crate::{api::utils::parse_uuid, parse_uuid};

use super::{mapping, TenantServiceComponents};

#[tonic::async_trait]
impl TenantsService for TenantServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn update_tenant(&self, request: Request<UpdateTenantRequest>) -> Result<Response<UpdateTenantResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;

        let inner = request.into_inner()
            .data
            .ok_or(TenantApiError::MissingArgument("No data provided".to_string()))?;
        ;

        let req = mapping::tenants::update_req_to_domain(inner);

        
        let organization = self.store.get_organization_by_id(organization_id).await.map_err(Into::<TenantApiError>::into)?;

        let res = self
            .store
            .update_tenant(req, organization_id, tenant_id)
            .await
            .map(mapping::tenants::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        invalidate_resolve_slugs_cache(&organization.slug, &res.slug).await;

        Ok(Response::new(UpdateTenantResponse {
            tenant: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn active_tenant(
        &self,
        request: Request<ActiveTenantRequest>,
    ) -> Result<Response<ActiveTenantResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;

        let tenant = self
            .store
            .find_tenant_by_id_and_organization(tenant_id, organization_id)
            .await
            .map(mapping::tenants::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        let organization = self
            .store
            .get_organization_by_id(organization_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;


        Ok(Response::new(ActiveTenantResponse {
            tenant: Some(tenant),
            trade_name: organization.trade_name,
            billing_config: None, // todo load it from provider_config if needed
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_tenants(
        &self,
        request: Request<ListTenantsRequest>,
    ) -> Result<Response<ListTenantsResponse>, Status> {
        let organization = request.organization()?;

        let result = self
            .store
            .list_tenants_by_organization_id(organization)
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
        let organization_id = request.organization()?;

        let req = request.into_inner();
        let tenant_id = parse_uuid!(&req.tenant_id)?;

        let tenant = self
            .store
            .find_tenant_by_id_and_organization(tenant_id, organization_id)
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
        let organization_id = request.organization()?;


        let req = mapping::tenants::create_req_to_domain(request.into_inner());

        let res = self
            .store
            .insert_tenant(req, organization_id)
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
