use common_domain::ids::{PlanId, ProductFamilyId};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_grpc::meteroid::api::pricingpage::v1::{
    GetPlanPricingPageRequest, GetPlanPricingPageResponse, GetPricingPageRequest,
    GetPricingPageResponse, UpdatePlanPricingPageRequest, UpdatePlanPricingPageResponse,
    pricing_page_service_server::PricingPageService,
};
use meteroid_store::domain::pricing_pages::PricingPageRequest;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::{PricingPagesInterface, ProductFamilyInterface};
use tonic::{Request, Response, Status};

use super::PricingPageServiceComponents;
use super::mapping;

fn store_status(report: Report<StoreError>) -> Status {
    match report.current_context() {
        StoreError::InvalidArgument(message) => Status::invalid_argument(message.clone()),
        StoreError::FailedPrecondition(message) => Status::failed_precondition(message.clone()),
        StoreError::ValueNotFound(message) => Status::not_found(message.clone()),
        StoreError::DuplicateValue { .. } => {
            Status::already_exists("Another plan of this product line was marked popular")
        }
        _ => Status::internal("Error in pricing page service"),
    }
}

#[tonic::async_trait]
impl PricingPageService for PricingPageServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_plan_pricing_page(
        &self,
        request: Request<GetPlanPricingPageRequest>,
    ) -> Result<Response<GetPlanPricingPageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let plan_id = PlanId::from_proto(&request.into_inner().plan_id)?;
        let page = self
            .store
            .get_plan_pricing_page(tenant_id, plan_id)
            .await
            .map_err(store_status)?;
        let charges = self
            .store
            .plan_pricing_charges(tenant_id, plan_id)
            .await
            .map_err(store_status)?;
        Ok(Response::new(GetPlanPricingPageResponse {
            page: Some(mapping::plan_page_to_proto(page)),
            charges: charges
                .charges
                .into_iter()
                .map(mapping::charge_to_proto)
                .collect(),
            product_family_id: charges.product_family_id.as_proto(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_plan_pricing_page(
        &self,
        request: Request<UpdatePlanPricingPageRequest>,
    ) -> Result<Response<UpdatePlanPricingPageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let page = request
            .into_inner()
            .page
            .ok_or_else(|| Status::invalid_argument("page is missing"))?;
        let page = self
            .store
            .update_plan_pricing_page(tenant_id, mapping::plan_page_from_proto(page)?)
            .await
            .map_err(store_status)?;
        Ok(Response::new(UpdatePlanPricingPageResponse {
            page: Some(mapping::plan_page_to_proto(page)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_pricing_page(
        &self,
        request: Request<GetPricingPageRequest>,
    ) -> Result<Response<GetPricingPageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let product_family_id = if inner.product_family_id.eq_ignore_ascii_case("default") {
            self.store
                .find_default_product_family(tenant_id)
                .await
                .map_err(store_status)?
                .id
        } else {
            ProductFamilyId::from_proto(&inner.product_family_id)?
        };
        let option = inner
            .option
            .map(|alias| alias.trim().to_string())
            .filter(|alias| !alias.is_empty());
        let draft = inner.draft.map(mapping::plan_page_from_proto).transpose()?;
        let page = self
            .store
            .pricing_page(
                tenant_id,
                product_family_id,
                PricingPageRequest {
                    currency: inner.currency,
                    option,
                    draft,
                },
            )
            .await
            .map_err(store_status)?;
        Ok(Response::new(GetPricingPageResponse {
            page: Some(mapping::page_to_proto(page)),
        }))
    }
}
