use crate::api::tenants::error::TenantApiError;
use common_domain::ids::TenantId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::tenants::v1::{
    ActiveTenantRequest, ActiveTenantResponse, CreateTenantRequest, CreateTenantResponse,
    GetTenantByIdRequest, GetTenantByIdResponse, ListTenantsCurrenciesRequest,
    ListTenantsCurrenciesResponse, ListTenantsCurrenciesWithCustomerCountRequest,
    ListTenantsCurrenciesWithCustomerCountResponse, ListTenantsRequest, ListTenantsResponse,
    UpdateTenantAvailableCurrenciesRequest, UpdateTenantAvailableCurrenciesResponse,
    UpdateTenantRequest, UpdateTenantResponse,
    list_tenants_currencies_with_customer_count_response::ListCurrency,
    tenants_service_server::TenantsService,
};
use meteroid_middleware::server::auth::strategies::jwt_strategy::invalidate_resolve_slugs_cache;
use meteroid_seeder::SeederInterface;
use meteroid_store::domain::TenantEnvironmentEnum;
use meteroid_store::repositories::tenants::invalidate_reporting_currency_cache;
use meteroid_store::repositories::{OrganizationsInterface, TenantInterface};
use tonic::{Request, Response, Status};

use super::{TenantServiceComponents, mapping};

#[tonic::async_trait]
impl TenantsService for TenantServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn update_tenant(
        &self,
        request: Request<UpdateTenantRequest>,
    ) -> Result<Response<UpdateTenantResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;

        let inner = request
            .into_inner()
            .data
            .ok_or(TenantApiError::MissingArgument(
                "No data provided".to_string(),
            ))?;

        let req = mapping::tenants::update_req_to_domain(inner);

        let organization = self
            .store
            .get_organization_by_id(organization_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        let res = self
            .store
            .update_tenant(req, organization_id, tenant_id)
            .await
            .map(mapping::tenants::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        invalidate_resolve_slugs_cache(&organization.slug, &res.slug).await;
        invalidate_reporting_currency_cache(&tenant_id).await;

        Ok(Response::new(UpdateTenantResponse { tenant: Some(res) }))
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
        let tenant_id = TenantId::from_proto(req.tenant_id)?;

        let tenant = self
            .store
            .find_tenant_by_id_and_organization(tenant_id, organization_id)
            .await
            .map(mapping::tenants::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(GetTenantByIdResponse {
            tenant: Some(tenant),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_tenant(
        &self,
        request: Request<CreateTenantRequest>,
    ) -> Result<Response<CreateTenantResponse>, Status> {
        let organization_id = request.organization()?;
        let actor = request.actor()?;

        let req = mapping::tenants::create_req_to_domain(request.into_inner());

        let res = if req.environment == TenantEnvironmentEnum::Sandbox {
            self.store
                .insert_seeded_sandbox_tenant(req.name, organization_id, actor)
                .await
                .map_err(Into::<TenantApiError>::into)?
        } else {
            self.store
                .insert_tenant(req, organization_id)
                .await
                .map_err(Into::<TenantApiError>::into)?
        };

        Ok(Response::new(CreateTenantResponse {
            tenant: Some(mapping::tenants::domain_to_server(res)),
        }))
    }

    async fn list_tenant_currencies(
        &self,
        request: Request<ListTenantsCurrenciesRequest>,
    ) -> Result<Response<ListTenantsCurrenciesResponse>, Status> {
        let tenant = request.tenant()?;

        let res = self
            .store
            .list_tenant_currencies(tenant)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(ListTenantsCurrenciesResponse {
            currencies: res,
        }))
    }

    async fn list_tenant_currencies_with_customer_count(
        &self,
        request: Request<ListTenantsCurrenciesWithCustomerCountRequest>,
    ) -> Result<Response<ListTenantsCurrenciesWithCustomerCountResponse>, Status> {
        let tenant = request.tenant()?;

        let res = self
            .store
            .list_tenant_currencies_with_customer_count(tenant)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(
            ListTenantsCurrenciesWithCustomerCountResponse {
                currencies: res
                    .into_iter()
                    .map(|(c, cc)| ListCurrency {
                        currency: c,
                        customer_count: cc,
                    })
                    .collect(),
            },
        ))
    }

    async fn update_tenant_available_currencies(
        &self,
        request: Request<UpdateTenantAvailableCurrenciesRequest>,
    ) -> Result<Response<UpdateTenantAvailableCurrenciesResponse>, Status> {
        let tenant = request.tenant()?;
        let res = self
            .store
            .update_tenant_available_currencies(tenant, request.into_inner().currencies)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(UpdateTenantAvailableCurrenciesResponse {
            currencies: res,
        }))
    }
}
