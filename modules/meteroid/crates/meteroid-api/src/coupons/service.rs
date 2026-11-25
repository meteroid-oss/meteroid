use crate::coupons::error::CouponApiError;
use crate::coupons::mapping::applied::AppliedCouponForDisplayWrapper;
use crate::coupons::mapping::coupons::CouponWrapper;
use crate::coupons::{CouponsServiceComponents, mapping};
use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, CouponId, PlanId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::conversions::FromProtoOpt;
use meteroid_grpc::meteroid::api::coupons::v1::coupons_service_server::CouponsService;
use meteroid_grpc::meteroid::api::coupons::v1::{
    CouponAction, CreateCouponRequest, CreateCouponResponse, EditCouponRequest, EditCouponResponse,
    GetCouponRequest, GetCouponResponse, ListAppliedCouponRequest, ListAppliedCouponResponse,
    ListCouponRequest, ListCouponResponse, RemoveCouponRequest, RemoveCouponResponse,
    UpdateCouponStatusRequest, UpdateCouponStatusResponse,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::domain;
use meteroid_store::repositories::coupons::CouponInterface;
use meteroid_store::repositories::coupons::CouponInterfaceAuto;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl CouponsService for CouponsServiceComponents {
    async fn get_coupon(
        &self,
        request: Request<GetCouponRequest>,
    ) -> Result<Response<GetCouponResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let coupon = self
            .store
            .get_coupon_by_id(tenant_id, CouponId::from_proto(req.coupon_local_id)?)
            .await
            .map(|x| CouponWrapper::from(x).0)
            .map_err(Into::<CouponApiError>::into)?;

        Ok(Response::new(GetCouponResponse {
            coupon: Some(coupon),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_coupons(
        &self,
        request: Request<ListCouponRequest>,
    ) -> Result<Response<ListCouponResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let filter = mapping::coupons::filter::from_server(req.filter());

        let pagination_req = req.pagination.into_domain();
        let plan_id = PlanId::from_proto_opt(req.plan_id)?;

        let coupons = self
            .store
            .list_coupons(
                tenant_id,
                pagination_req,
                req.search,
                filter,
                req.order_by,
                plan_id,
            )
            .await
            .map_err(Into::<CouponApiError>::into)?;

        let response = ListCouponResponse {
            pagination_meta: req
                .pagination
                .into_response(coupons.total_pages, coupons.total_results),
            coupons: coupons
                .items
                .into_iter()
                .map(|x| CouponWrapper::from(x).0)
                .collect(),
        };

        Ok(Response::new(response))
    }

    async fn list_applied_coupons(
        &self,
        request: Request<ListAppliedCouponRequest>,
    ) -> Result<Response<ListAppliedCouponResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let pagination_req = req.pagination.into_domain();

        let coupons = self
            .store
            .list_applied_coupons_by_coupon_id(
                tenant_id,
                CouponId::from_proto(req.coupon_local_id)?,
                pagination_req,
            )
            .await
            .map_err(Into::<CouponApiError>::into)?;

        let response = ListAppliedCouponResponse {
            pagination_meta: req
                .pagination
                .into_response(coupons.total_pages, coupons.total_results),
            applied_coupons: coupons
                .items
                .into_iter()
                .map(|x| AppliedCouponForDisplayWrapper::from(x).0)
                .collect(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_coupon(
        &self,
        request: Request<CreateCouponRequest>,
    ) -> Result<Response<CreateCouponResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;

        let req = request.into_inner();

        let fixed_amounts =
            mapping::coupons::discount::fixed_amounts_to_domain(req.discount.as_ref())?;
        let discount = mapping::coupons::discount::to_domain(req.discount)?;

        let plan_ids = req
            .plan_ids
            .into_iter()
            .map(|id| PlanId::from_proto(&id))
            .collect::<Result<Vec<_>, _>>()?;
        let terms = domain::coupons::CouponTerms {
            add_on_ids: add_on_ids_from_proto(req.add_on_ids)?,
            eligibility: req.eligibility.map(crate::eligibility::from_proto),
            valid_from: NaiveDateTime::from_proto_opt(req.valid_from.filter(|v| !v.is_empty()))?,
            fixed_amounts,
            duration_months: req.duration_months,
        };

        let new = domain::coupons::CouponNew {
            code: req.code,
            description: req.description,
            discount,
            expires_at: NaiveDateTime::from_proto_opt(req.expires_at)?,
            redemption_limit: req.redemption_limit,
            tenant_id,
            recurring_value: req.recurring_value,
            reusable: req.reusable,
            plan_ids,
        };

        let added = self
            .store
            .create_coupon_with_terms(actor, new, terms)
            .await
            .map(|x| CouponWrapper::from(x).0)
            .map_err(Into::<CouponApiError>::into)?;

        Ok(Response::new(CreateCouponResponse {
            coupon: Some(added),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn remove_coupon(
        &self,
        request: Request<RemoveCouponRequest>,
    ) -> Result<Response<RemoveCouponResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let id = CouponId::from_proto(&req.coupon_id)?;

        self.store
            .delete_coupon(tenant_id, id)
            .await
            .map_err(Into::<CouponApiError>::into)?;

        Ok(Response::new(RemoveCouponResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn edit_coupon(
        &self,
        request: Request<EditCouponRequest>,
    ) -> Result<Response<EditCouponResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;

        let req = request.into_inner();

        let fixed_amounts =
            mapping::coupons::discount::fixed_amounts_to_domain(req.discount.as_ref())?;
        let discount = mapping::coupons::discount::to_domain(req.discount)?;

        let plan_ids = req
            .plan_ids
            .into_iter()
            .map(|id| PlanId::from_proto(&id))
            .collect::<Result<Vec<_>, _>>()?;

        let duration_months = match (req.clear_duration_months, req.duration_months) {
            (true, Some(_)) => {
                return Err(Status::invalid_argument(
                    "a coupon lasts forever or a number of months, not both",
                ));
            }
            (true, None) => Some(None),
            (false, months) => months.map(Some),
        };

        let patch = domain::coupons::CouponPatch {
            id: CouponId::from_proto(&req.coupon_id)?,
            tenant_id,
            description: Some(req.description),
            discount: Some(discount),
            plan_ids: Some(plan_ids),
            add_on_ids: Some(add_on_ids_from_proto(req.add_on_ids)?),
            eligibility: req
                .eligibility
                .map(|rule| Some(crate::eligibility::from_proto(rule))),
            valid_from: moment_patch(req.valid_from)?,
            expires_at: moment_patch(req.expires_at)?,
            fixed_amounts: Some(fixed_amounts),
            duration_months,
        };

        let updated = self
            .store
            .update_coupon(actor, patch)
            .await
            .map(|x| CouponWrapper::from(x).0)
            .map_err(Into::<CouponApiError>::into)?;

        Ok(Response::new(EditCouponResponse {
            coupon: Some(updated),
        }))
    }

    async fn update_coupon_status(
        &self,
        request: Request<UpdateCouponStatusRequest>,
    ) -> Result<Response<UpdateCouponStatusResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;

        let req = request.into_inner();

        let id = CouponId::from_proto(&req.coupon_id)?;

        let (archived_at, disabled) = match req.action() {
            CouponAction::Archive => {
                let now = common_clock::now().naive_utc();
                (Some(Some(now)), None)
            }
            CouponAction::Disable => (None, Some(true)),
            CouponAction::Enable => (Some(None), Some(false)),
        };

        let patch = domain::coupons::CouponStatusPatch {
            id,
            tenant_id,
            archived_at,
            disabled,
        };

        let updated = self
            .store
            .update_coupon_status(actor, patch)
            .await
            .map(|x| CouponWrapper::from(x).0)
            .map_err(Into::<CouponApiError>::into)?;

        Ok(Response::new(UpdateCouponStatusResponse {
            coupon: Some(updated),
        }))
    }
}

fn add_on_ids_from_proto(ids: Vec<String>) -> Result<Vec<AddOnId>, Status> {
    ids.iter().map(AddOnId::from_proto).collect()
}

/// Unset leaves the moment as it is; an empty one clears it.
fn moment_patch(value: Option<String>) -> Result<Option<Option<NaiveDateTime>>, Status> {
    match value {
        None => Ok(None),
        Some(value) if value.is_empty() => Ok(Some(None)),
        Some(value) => NaiveDateTime::from_proto_opt(Some(value)).map(Some),
    }
}
