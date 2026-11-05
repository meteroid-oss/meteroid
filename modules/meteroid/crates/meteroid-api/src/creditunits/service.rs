use common_domain::ids::{CreditUnitId, CreditUnitPriceId, ProductId, TenantId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_grpc::meteroid::api::creditunits::v1::{
    ArchiveCreditUnitPriceRequest, ArchiveCreditUnitPriceResponse, ArchiveCreditUnitRequest,
    ArchiveCreditUnitResponse, CreateCreditUnitRequest, CreateCreditUnitResponse, CreditUnit,
    GetCreditUnitRequest, GetCreditUnitResponse, ListCreditUnitPricesRequest,
    ListCreditUnitPricesResponse, ListCreditUnitsRequest, ListCreditUnitsResponse,
    SetCreditUnitPriceRequest, SetCreditUnitPriceResponse, SetCreditUnitProductRequest,
    SetCreditUnitProductResponse, UpdateCreditUnitRequest, UpdateCreditUnitResponse,
    credit_units_service_server::CreditUnitsService,
};
use meteroid_store::domain::credit_units::{self as domain, CreditUnitPatch};
use meteroid_store::repositories::CreditUnitsInterface;
use tonic::{Request, Response, Status};

use crate::creditunits::error::CreditUnitsApiError;

use super::{CreditUnitsServiceComponents, mapping::credit_units as mapping};

impl CreditUnitsServiceComponents {
    /// The unit with its active selling prices.
    async fn view(
        &self,
        tenant: TenantId,
        unit: domain::CreditUnit,
    ) -> Result<CreditUnit, CreditUnitsApiError> {
        let prices = self
            .store
            .list_credit_unit_prices(tenant, vec![unit.id], false)
            .await?;
        Ok(mapping::domain_to_proto(unit, &prices))
    }
}

#[tonic::async_trait]
impl CreditUnitsService for CreditUnitsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_credit_units(
        &self,
        request: Request<ListCreditUnitsRequest>,
    ) -> Result<Response<ListCreditUnitsResponse>, Status> {
        let tenant = request.tenant()?;
        let include_archived = request.get_ref().include_archived;

        let units = self
            .store
            .list_credit_units(tenant, include_archived)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;
        let prices = self
            .store
            .list_credit_unit_prices(tenant, units.iter().map(|unit| unit.id).collect(), false)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;
        let units = units
            .into_iter()
            .map(|unit| mapping::domain_to_proto(unit, &prices))
            .collect();

        Ok(Response::new(ListCreditUnitsResponse { units }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_credit_unit(
        &self,
        request: Request<GetCreditUnitRequest>,
    ) -> Result<Response<GetCreditUnitResponse>, Status> {
        let tenant = request.tenant()?;
        let id = CreditUnitId::from_proto(request.into_inner().id)?;

        let unit = self
            .store
            .get_credit_unit(tenant, id)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;

        Ok(Response::new(GetCreditUnitResponse {
            unit: Some(self.view(tenant, unit).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_credit_unit(
        &self,
        request: Request<CreateCreditUnitRequest>,
    ) -> Result<Response<CreateCreditUnitResponse>, Status> {
        let tenant = request.tenant()?;
        let organization = request.organization()?;
        let new = mapping::create_req_to_domain(tenant, request.into_inner())?;
        ensure_credit_pricing_allowed(self.feature_resolver.as_ref(), organization, true).await?;

        let unit = self
            .store
            .create_credit_unit(new)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;

        Ok(Response::new(CreateCreditUnitResponse {
            unit: Some(self.view(tenant, unit).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_credit_unit(
        &self,
        request: Request<UpdateCreditUnitRequest>,
    ) -> Result<Response<UpdateCreditUnitResponse>, Status> {
        let tenant = request.tenant()?;
        let organization = request.organization()?;
        let patch = mapping::update_req_to_domain(tenant, request.into_inner())?;
        ensure_credit_pricing_allowed(
            self.feature_resolver.as_ref(),
            organization,
            patch.archived == Some(false),
        )
        .await?;

        let unit = self
            .store
            .update_credit_unit(patch)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;

        Ok(Response::new(UpdateCreditUnitResponse {
            unit: Some(self.view(tenant, unit).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn archive_credit_unit(
        &self,
        request: Request<ArchiveCreditUnitRequest>,
    ) -> Result<Response<ArchiveCreditUnitResponse>, Status> {
        let tenant = request.tenant()?;
        let id = CreditUnitId::from_proto(request.into_inner().id)?;

        let patch = CreditUnitPatch::new(tenant, id, None, None, None, Some(true))
            .map_err(Into::<CreditUnitsApiError>::into)?;
        let unit = self
            .store
            .update_credit_unit(patch)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;

        Ok(Response::new(ArchiveCreditUnitResponse {
            unit: Some(self.view(tenant, unit).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_credit_unit_prices(
        &self,
        request: Request<ListCreditUnitPricesRequest>,
    ) -> Result<Response<ListCreditUnitPricesResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();
        let id = CreditUnitId::from_proto(req.credit_unit_id)?;
        self.store
            .get_credit_unit(tenant, id)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;
        let prices = self
            .store
            .list_credit_unit_prices(tenant, vec![id], req.include_archived)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;
        Ok(Response::new(ListCreditUnitPricesResponse {
            prices: prices.iter().map(mapping::price_to_proto).collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_credit_unit_price(
        &self,
        request: Request<SetCreditUnitPriceRequest>,
    ) -> Result<Response<SetCreditUnitPriceResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();
        let id = CreditUnitId::from_proto(req.credit_unit_id)?;
        let unit_price = mapping::parse_unit_price(&req.unit_price)?;
        self.store
            .set_credit_unit_price(tenant, id, &req.currency, unit_price)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;
        let unit = self
            .store
            .get_credit_unit(tenant, id)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;
        Ok(Response::new(SetCreditUnitPriceResponse {
            unit: Some(self.view(tenant, unit).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn archive_credit_unit_price(
        &self,
        request: Request<ArchiveCreditUnitPriceRequest>,
    ) -> Result<Response<ArchiveCreditUnitPriceResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();
        let id = CreditUnitId::from_proto(req.credit_unit_id)?;
        self.store
            .archive_credit_unit_price(tenant, id, CreditUnitPriceId::from_proto(req.price_id)?)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;
        let unit = self
            .store
            .get_credit_unit(tenant, id)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;
        Ok(Response::new(ArchiveCreditUnitPriceResponse {
            unit: Some(self.view(tenant, unit).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_credit_unit_product(
        &self,
        request: Request<SetCreditUnitProductRequest>,
    ) -> Result<Response<SetCreditUnitProductResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();
        let id = CreditUnitId::from_proto(req.credit_unit_id)?;
        let product_id = req.product_id.map(ProductId::from_proto).transpose()?;
        let patch = CreditUnitPatch::new(tenant, id, None, None, None, None)
            .map_err(Into::<CreditUnitsApiError>::into)?
            .with_product_id(product_id);
        let unit = self
            .store
            .update_credit_unit(patch)
            .await
            .map_err(Into::<CreditUnitsApiError>::into)?;
        Ok(Response::new(SetCreditUnitProductResponse {
            unit: Some(self.view(tenant, unit).await?),
        }))
    }
}
