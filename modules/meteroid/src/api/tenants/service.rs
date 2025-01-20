use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::tenants::v1::{
    tenants_service_server::TenantsService, ActiveTenantRequest, ActiveTenantResponse,
    AddTenantCurrencyRequest, AddTenantCurrencyResponse, CreateTenantRequest, CreateTenantResponse,
    GetTenantByIdRequest, GetTenantByIdResponse, ListTenantsCurrenciesRequest,
    ListTenantsCurrenciesResponse, ListTenantsRequest, ListTenantsResponse,
    RemoveTenantCurrencyRequest, RemoveTenantCurrencyResponse, UpdateTenantRequest,
    UpdateTenantResponse,
};
use meteroid_middleware::server::auth::strategies::jwt_strategy::invalidate_resolve_slugs_cache;
use meteroid_store::repositories::tenants::invalidate_reporting_currency_cache;
use meteroid_store::repositories::{OrganizationsInterface, TenantInterface};

use crate::api::tenants::error::TenantApiError;
use crate::{api::utils::parse_uuid, parse_uuid};

use super::{mapping, TenantServiceComponents};

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
        let tenant_id = parse_uuid!(&req.tenant_id)?;

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

    async fn add_tenant_currency(
        &self,
        request: Request<AddTenantCurrencyRequest>,
    ) -> Result<Response<AddTenantCurrencyResponse>, Status> {
        let tenant = request.tenant()?;
        self.store
            .add_tenant_currency(tenant, request.into_inner().currency)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(AddTenantCurrencyResponse {}))
    }

    async fn remove_tenant_currency(
        &self,
        request: Request<RemoveTenantCurrencyRequest>,
    ) -> Result<Response<RemoveTenantCurrencyResponse>, Status> {
        let tenant = request.tenant()?;
        self.store
            .remove_tenant_currency(tenant, request.into_inner().currency)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(RemoveTenantCurrencyResponse {}))
    }
}
