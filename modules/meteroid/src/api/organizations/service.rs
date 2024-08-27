use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::organizations::v1::{
    organizations_service_server::OrganizationsService, CreateOrganizationRequest,
    CreateOrganizationResponse, GetCurrentOrganizationRequest, GetCurrentOrganizationResponse,
    ListOrganizationsRequest, ListOrganizationsResponse, Organization,
};
use meteroid_store::domain::OrganizationNew;
use meteroid_store::repositories::organizations::OrganizationsInterface;

use crate::api::organizations::error::OrganizationApiError;

use super::{mapping, OrganizationsServiceComponents};

#[tonic::async_trait]
impl OrganizationsService for OrganizationsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_organizations(
        &self,
        request: Request<ListOrganizationsRequest>,
    ) -> Result<Response<ListOrganizationsResponse>, Status> {
        let user = request.actor()?;

        let organizations: Vec<Organization> = self
            .store
            .list_organizations_for_user(user)
            .await
            .map_err(Into::<OrganizationApiError>::into)?
            .into_iter()
            .map(mapping::organization::domain_to_proto)
            .collect();

        let response = ListOrganizationsResponse { organizations };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_current_organizations(
        &self,
        request: Request<GetCurrentOrganizationRequest>,
    ) -> Result<Response<GetCurrentOrganizationResponse>, Status> {
        let organization_id = request.organization()?;
        let organization = self
            .store
            .get_organizations_with_tenants_by_id(organization_id)
            .await
            .map_err(Into::<OrganizationApiError>::into)?;

        let response = GetCurrentOrganizationResponse {
            organization: Some(mapping::organization::domain_with_tenants_to_proto(
                organization,
            )),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_organization(
        &self,
        request: Request<CreateOrganizationRequest>,
    ) -> Result<Response<CreateOrganizationResponse>, Status> {
        let user = request.actor()?;
        let request = request.into_inner();

        let organization_new = OrganizationNew {
            trade_name: request.trade_name.clone(),
            country: request.country.clone(),
            invoicing_entity: None,
        };

        let organization = self
            .store
            .insert_organization(organization_new, user)
            .await
            .map_err(Into::<OrganizationApiError>::into)?;

        let response = CreateOrganizationResponse {
            organization: Some(mapping::organization::domain_with_tenants_to_proto(
                organization,
            )),
        };

        Ok(Response::new(response))
    }
}
