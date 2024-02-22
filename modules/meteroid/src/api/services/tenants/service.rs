use std::sync::Arc;

use meteroid_grpc::meteroid::api::tenants::v1::tenant_billing_configuration::BillingConfigOneof;
use meteroid_grpc::meteroid::api::tenants::v1::{
    tenants_service_server::TenantsService, ActiveTenantRequest, ActiveTenantResponse,
    ConfigureTenantBillingRequest, ConfigureTenantBillingResponse, CreateTenantRequest,
    CreateTenantResponse, GetTenantByIdRequest, GetTenantByIdResponse, ListTenantsRequest,
    ListTenantsResponse, Tenant,
};
use meteroid_repository as db;
use tonic::{Request, Response, Status};

use crate::repo::provider_config::model::InvoicingProvider;
use crate::{
    api::services::utils::{parse_uuid, uuid_gen},
    parse_uuid,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_repository::Params;

use super::{mapping, TenantServiceComponents};

#[tonic::async_trait]
impl TenantsService for TenantServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn active_tenant(
        &self,
        request: Request<ActiveTenantRequest>,
    ) -> Result<Response<ActiveTenantResponse>, Status> {
        let tenant_id = request.tenant()?;
        let connection = self.get_connection().await?;

        let db_tenant = db::tenants::get_tenant_by_id()
            .bind(&connection, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to get tenant by id")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        Ok(Response::new(ActiveTenantResponse {
            tenant: Some(Tenant {
                id: db_tenant.id.to_string(),
                name: db_tenant.name,
                slug: db_tenant.slug,
                currency: db_tenant.currency,
            }),
            billing_config: None, // todo load it from provider_config if needed
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_tenants(
        &self,
        request: Request<ListTenantsRequest>,
    ) -> Result<Response<ListTenantsResponse>, Status> {
        let connection = self.get_connection().await?;

        let tenants = db::tenants::tenants_per_user()
            .bind(&connection, &request.actor()?)
            .all()
            .await
            .map_err(|e| {
                Status::internal("Failed to get tenants for user")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let result = tenants
            .into_iter()
            .map(mapping::tenants::db_to_server)
            .collect::<Vec<_>>();

        Ok(Response::new(ListTenantsResponse { tenants: result }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_tenant_by_id(
        &self,
        request: Request<GetTenantByIdRequest>,
    ) -> Result<Response<GetTenantByIdResponse>, Status> {
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let db_tenant = db::tenants::get_tenant_by_id()
            .bind(&connection, &parse_uuid!(&req.tenant_id)?)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to get tenant by id")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        Ok(Response::new(GetTenantByIdResponse {
            tenant: Some(Tenant {
                id: db_tenant.id.to_string(),
                name: db_tenant.name,
                slug: db_tenant.slug,
                currency: db_tenant.currency,
            }),
            billing_config: None, // todo load it from provider_config if needed
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_tenant(
        &self,
        request: Request<CreateTenantRequest>,
    ) -> Result<Response<CreateTenantResponse>, Status> {
        let actor = request.actor()?;

        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let params = db::tenants::CreateTenantForUserParams {
            id: uuid_gen::v7(),
            name: req.name,
            slug: req.slug,
            currency: req.currency,
            user_id: actor,
        };

        let tenant = db::tenants::create_tenant_for_user()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to create tenant")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let rs = mapping::tenants::db_to_server(tenant);
        Ok(Response::new(CreateTenantResponse { tenant: Some(rs) }))
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
            .ok_or(Status::invalid_argument("Missing billing_config"))?
            .billing_config_oneof
            .ok_or(Status::invalid_argument("Missing billing_config_oneof"))?;

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
                        Status::internal("Failed to create tenant billing_config")
                            .set_source(Arc::new(e.into_error()))
                            .clone()
                    })?;

                Ok(Response::new(ConfigureTenantBillingResponse {
                    billing_config: mapping::provider_configs::db_to_server(cfg),
                }))
            }
        }
    }
}
