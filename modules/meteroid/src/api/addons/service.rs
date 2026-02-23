use crate::api::addons::AddOnsServiceComponents;
use crate::api::addons::error::AddOnApiError;
use crate::api::addons::mapping::addons::{AddOnWrapper, PlanVersionAddOnWrapper};
use crate::api::pricecomponents::mapping::components::{
    price_entries_from_proto, product_ref_from_proto,
};
use crate::api::utils::PaginationExt;
use common_domain::ids::{AddOnId, PlanVersionId, PriceId, ProductFamilyId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::addons::v1::add_ons_service_server::AddOnsService;
use meteroid_grpc::meteroid::api::addons::v1::{
    AttachAddOnToPlanVersionRequest, AttachAddOnToPlanVersionResponse, CreateAddOnRequest,
    CreateAddOnResponse, DetachAddOnFromPlanVersionRequest, DetachAddOnFromPlanVersionResponse,
    EditAddOnRequest, EditAddOnResponse, GetAddOnRequest, GetAddOnResponse, ListAddOnRequest,
    ListAddOnResponse, ListPlanVersionAddOnsRequest, ListPlanVersionAddOnsResponse,
    RemoveAddOnRequest, RemoveAddOnResponse,
};
use meteroid_store::domain::add_ons::AddOnPatch;
use meteroid_store::domain::plan_version_add_ons::PlanVersionAddOnNew;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::plan_version_add_ons::PlanVersionAddOnInterface;
use meteroid_store::repositories::product_families::ProductFamilyInterface;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl AddOnsService for AddOnsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_add_ons(
        &self,
        request: Request<ListAddOnRequest>,
    ) -> Result<Response<ListAddOnResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = req
            .plan_version_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(PlanVersionId::from_proto)
            .transpose()?;
        let pagination_req = req.pagination.into_domain();

        let add_ons = self
            .store
            .list_add_ons(tenant_id, plan_version_id, pagination_req, req.search)
            .await
            .map_err(Into::<AddOnApiError>::into)?;

        let response = ListAddOnResponse {
            pagination_meta: req
                .pagination
                .into_response(add_ons.total_pages, add_ons.total_results),
            add_ons: add_ons
                .items
                .into_iter()
                .map(|x| AddOnWrapper::from(x).0)
                .collect(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_add_on(
        &self,
        request: Request<CreateAddOnRequest>,
    ) -> Result<Response<CreateAddOnResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let product_ref = product_ref_from_proto(req.product)?;
        let price_entries = price_entries_from_proto(req.price.into_iter().collect())?;
        let price_entry = price_entries
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("price is required"))?;

        let pf_id = match req.product_family_local_id.filter(|s| !s.is_empty()) {
            Some(id) => ProductFamilyId::from_proto(id)?,
            None => {
                self.store
                    .find_default_product_family(tenant_id)
                    .await
                    .map_err(Into::<AddOnApiError>::into)?
                    .id
            }
        };

        let added = self
            .store
            .create_add_on_from_ref(
                req.name,
                product_ref,
                price_entry,
                req.description,
                req.self_serviceable,
                req.max_instances_per_subscription,
                tenant_id,
                actor,
                pf_id,
            )
            .await
            .map(|x| AddOnWrapper::from(x).0)
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(CreateAddOnResponse {
            add_on: Some(added),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_add_on(
        &self,
        request: Request<GetAddOnRequest>,
    ) -> Result<Response<GetAddOnResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let add_on_id = AddOnId::from_proto(&req.add_on_id)?;

        let add_on = self
            .store
            .get_add_on_by_id(tenant_id, add_on_id)
            .await
            .map(|x| AddOnWrapper::from(x).0)
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(GetAddOnResponse {
            add_on: Some(add_on),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn remove_add_on(
        &self,
        request: Request<RemoveAddOnRequest>,
    ) -> Result<Response<RemoveAddOnResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let add_on_id = AddOnId::from_proto(&req.add_on_id)?;

        self.store
            .archive_add_on(add_on_id, tenant_id)
            .await
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(RemoveAddOnResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn edit_add_on(
        &self,
        request: Request<EditAddOnRequest>,
    ) -> Result<Response<EditAddOnResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let add_on_id = AddOnId::from_proto(&req.add_on_id)?;

        let price_entry = if let Some(entry) = req.price {
            let entries = price_entries_from_proto(vec![entry])?;
            entries.into_iter().next()
        } else {
            None
        };

        let name = if req.name.is_empty() {
            None
        } else {
            Some(req.name)
        };

        let patch = AddOnPatch {
            id: add_on_id,
            tenant_id,
            name,
            description: req.description.map(Some),
            self_serviceable: req.self_serviceable,
            max_instances_per_subscription: req.max_instances_per_subscription.map(Some),
        };

        let edited = self
            .store
            .update_add_on(patch, price_entry, actor)
            .await
            .map(|x| AddOnWrapper::from(x).0)
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(EditAddOnResponse {
            add_on: Some(edited),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn attach_add_on_to_plan_version(
        &self,
        request: Request<AttachAddOnToPlanVersionRequest>,
    ) -> Result<Response<AttachAddOnToPlanVersionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;
        let add_on_id = AddOnId::from_proto(&req.add_on_id)?;
        let price_id = req
            .price_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(PriceId::from_proto)
            .transpose()?;

        let new = PlanVersionAddOnNew {
            plan_version_id,
            add_on_id,
            price_id,
            self_serviceable: req.self_serviceable,
            max_instances_per_subscription: req.max_instances_per_subscription,
            tenant_id,
        };

        let result = self
            .store
            .attach_add_on_to_plan_version(new)
            .await
            .map(|x| PlanVersionAddOnWrapper::from(x).0)
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(AttachAddOnToPlanVersionResponse {
            plan_version_add_on: Some(result),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn detach_add_on_from_plan_version(
        &self,
        request: Request<DetachAddOnFromPlanVersionRequest>,
    ) -> Result<Response<DetachAddOnFromPlanVersionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;
        let add_on_id = AddOnId::from_proto(&req.add_on_id)?;

        self.store
            .detach_add_on_from_plan_version(plan_version_id, add_on_id, tenant_id)
            .await
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(DetachAddOnFromPlanVersionResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn list_plan_version_add_ons(
        &self,
        request: Request<ListPlanVersionAddOnsRequest>,
    ) -> Result<Response<ListPlanVersionAddOnsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        let result = self
            .store
            .list_plan_version_add_ons(plan_version_id, tenant_id)
            .await
            .map_err(Into::<AddOnApiError>::into)?;

        Ok(Response::new(ListPlanVersionAddOnsResponse {
            plan_version_add_ons: result
                .into_iter()
                .map(|x| PlanVersionAddOnWrapper::from(x).0)
                .collect(),
        }))
    }
}
