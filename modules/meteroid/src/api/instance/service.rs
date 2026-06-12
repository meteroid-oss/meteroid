use tonic::{Request, Response, Status};

use common_domain::country::CountryCode;
use common_domain::ids::OrganizationInviteId;
use meteroid_grpc::meteroid::api::instance::v1::get_countries_response::Country as GrpcCountry;
use meteroid_grpc::meteroid::api::instance::v1::get_currencies_response::Currency as GrpcCurrency;
use meteroid_grpc::meteroid::api::instance::v1::get_subdivisions_response::Subdivision as GrpcSubdivision;
use meteroid_grpc::meteroid::api::instance::v1::instance_service_server::InstanceService;
use meteroid_grpc::meteroid::api::instance::v1::{
    GetCountriesRequest, GetCountriesResponse, GetCurrenciesRequest, GetCurrenciesResponse,
    GetInstanceRequest, GetInstanceResponse, GetInviteDetailsRequest, GetInviteDetailsResponse,
    GetSubdivisionsRequest, GetSubdivisionsResponse,
};
use meteroid_store::constants::{COUNTRIES, CURRENCIES};
use meteroid_store::repositories::OrganizationsInterface;

use crate::api::instance::InstanceServiceComponents;
use crate::api::instance::error::InstanceApiError;

#[tonic::async_trait]
impl InstanceService for InstanceServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_instance(
        &self,
        _request: Request<GetInstanceRequest>,
    ) -> Result<Response<GetInstanceResponse>, Status> {
        let maybe_instance = self
            .store
            .get_instance()
            .await
            .map_err(Into::<InstanceApiError>::into)?;

        Ok(Response::new(GetInstanceResponse {
            multi_organization_enabled: maybe_instance.multi_organization_enabled,
            instance_initiated: maybe_instance.instance_initiated,
            mailer_enabled: maybe_instance.mailer_enabled,
            google_oauth_client_id: maybe_instance.google_oauth_client_id,
            hubspot_oauth_client_id: maybe_instance.hubspot_oauth_client_id,
            pennylane_oauth_client_id: maybe_instance.pennylane_oauth_client_id,
            svix_enabled: self.svix_enabled,
        }))
    }

    async fn get_invite_details(
        &self,
        request: Request<GetInviteDetailsRequest>,
    ) -> Result<Response<GetInviteDetailsResponse>, Status> {
        let req = request.into_inner();

        let invite_id = OrganizationInviteId::from_proto(&req.invite_id)
            .map_err(|_| Status::invalid_argument("Invalid invite_id"))?;

        let details = self
            .store
            .get_invite_details(invite_id)
            .await
            .map_err(Into::<InstanceApiError>::into)?;

        Ok(Response::new(GetInviteDetailsResponse {
            organization_name: details.organization_name,
            role: crate::api::users::mapping::role::domain_to_server(details.role.into()).into(),
            invited_email: details.invited_email,
        }))
    }

    async fn get_countries(
        &self,
        _request: Request<GetCountriesRequest>,
    ) -> Result<Response<GetCountriesResponse>, Status> {
        let countries = COUNTRIES
            .iter()
            .map(|country| GrpcCountry {
                code: country.code.to_string(),
                name: country.name.to_string(),
                currency: country.currency.to_string(),
            })
            .collect();

        Ok(Response::new(GetCountriesResponse { countries }))
    }

    async fn get_currencies(
        &self,
        _request: Request<GetCurrenciesRequest>,
    ) -> Result<Response<GetCurrenciesResponse>, Status> {
        let currencies = CURRENCIES
            .iter()
            .map(|currency| GrpcCurrency {
                code: currency.code.to_string(),
                name: currency.name.to_string(),
                symbol: currency.symbol.to_string(),
                precision: u32::from(currency.precision),
            })
            .collect();

        Ok(Response::new(GetCurrenciesResponse { currencies }))
    }

    async fn get_subdivisions(
        &self,
        request: Request<GetSubdivisionsRequest>,
    ) -> Result<Response<GetSubdivisionsResponse>, Status> {
        let country_code = request.into_inner().country_code;

        let country = CountryCode::from_proto(&country_code)?;

        let subdivisions = country
            .subdivisions()
            .into_iter()
            .map(|subdivision| GrpcSubdivision {
                code: subdivision.code,
                name: subdivision.name,
            })
            .collect();

        Ok(Response::new(GetSubdivisionsResponse { subdivisions }))
    }
}
