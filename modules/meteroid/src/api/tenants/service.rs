use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::tenants::v1::tenant_billing_configuration::BillingConfigOneof;
use meteroid_grpc::meteroid::api::tenants::v1::{
    tenants_service_server::TenantsService, ActiveTenantRequest, ActiveTenantResponse,
    ConfigureTenantBillingRequest, ConfigureTenantBillingResponse, CreateTenantRequest,
    CreateTenantResponse, GetTenantByIdRequest, GetTenantByIdResponse, ListTenantsRequest,
    ListTenantsResponse,
};
use meteroid_store::repositories::TenantInterface;

use crate::api::tenants::error::TenantApiError;
use crate::repo::provider_config::model::InvoicingProvider;
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
            .map(mapping::tenants::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(CreateTenantResponse { tenant: Some(res) }))
    }

    #[tracing::instrument(skip_all)]
    async fn configure_tenant_billing(
        &self,
        request: Request<ConfigureTenantBillingRequest>,
    ) -> Result<Response<ConfigureTenantBillingResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let billing_config = req
            .billing_config
            .clone()
            .ok_or(TenantApiError::MissingArgument(
                "billing_config".to_string(),
            ))?
            .billing_config_oneof
            .ok_or(TenantApiError::MissingArgument(
                "billing_config_oneof".to_string(),
            ))?;

        match billing_config {
            BillingConfigOneof::Stripe(stripe) => {
                let wh_secret = secrecy::SecretString::new(stripe.webhook_secret);
                let api_secret = secrecy::SecretString::new(stripe.api_secret);

                let cfg = self
                    .provider_config_repo
                    .create_provider_config(
                        InvoicingProvider::Stripe,
                        tenant_id,
                        api_secret,
                        wh_secret,
                    )
                    .await
                    .map_err(|e| {
                        TenantApiError::DownstreamApiError(
                            "failed to create tenant billing_config".to_string(),
                            Box::new(e.into_error()),
                        )
                    })?;

                Ok(Response::new(ConfigureTenantBillingResponse {
                    billing_config: mapping::provider_configs::db_to_server(cfg),
                }))
            }
        }
    }
}
