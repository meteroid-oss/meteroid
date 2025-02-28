use crate::api::coupons::error::CouponApiError;
use crate::api::coupons::mapping::applied::AppliedCouponForDisplayWrapper;
use crate::api::coupons::mapping::coupons::CouponWrapper;
use crate::api::coupons::{mapping, CouponsServiceComponents};
use crate::api::shared::conversions::FromProtoOpt;
use crate::api::utils::PaginationExt;
use chrono::NaiveDateTime;
use common_domain::ids::CouponId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::coupons::v1::coupons_service_server::CouponsService;
use meteroid_grpc::meteroid::api::coupons::v1::{
    CouponAction, CreateCouponRequest, CreateCouponResponse, EditCouponRequest, EditCouponResponse,
    GetCouponRequest, GetCouponResponse, ListAppliedCouponRequest, ListAppliedCouponResponse,
    ListCouponRequest, ListCouponResponse, RemoveCouponRequest, RemoveCouponResponse,
    UpdateCouponStatusRequest, UpdateCouponStatusResponse,
};
use meteroid_store::domain;
use meteroid_store::repositories::coupons::CouponInterface;
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

        let pagination_req = domain::PaginationRequest {
            page: req.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: req.pagination.as_ref().map(|p| p.limit),
        };

        let coupons = self
            .store
            .list_coupons(tenant_id, pagination_req, req.search, filter)
            .await
            .map_err(Into::<CouponApiError>::into)?;

        let response = ListCouponResponse {
            pagination_meta: req.pagination.into_response(coupons.total_results as u32),
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

        let pagination_req = domain::PaginationRequest {
            page: req.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: req.pagination.as_ref().map(|p| p.limit),
        };

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
            pagination_meta: req.pagination.into_response(coupons.total_results as u32),
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

        let req = request.into_inner();

        let discount = mapping::coupons::discount::to_domain(req.discount)?;

        let new = domain::coupons::CouponNew {
            code: req.code,
            description: req.description,
            discount,
            expires_at: NaiveDateTime::from_proto_opt(req.expires_at)?,
            redemption_limit: req.redemption_limit,
            tenant_id,
            recurring_value: req.recurring_value,
            reusable: req.reusable,
        };

        let added = self
            .store
            .create_coupon(new)
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

        let req = request.into_inner();

        let discount = mapping::coupons::discount::to_domain(req.discount)?;

        let patch = domain::coupons::CouponPatch {
            id: CouponId::from_proto(&req.coupon_id)?,
            tenant_id,
            description: Some(req.description),
            discount: Some(discount),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        let updated = self
            .store
            .update_coupon(patch)
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

        let req = request.into_inner();

        let id = CouponId::from_proto(&req.coupon_id)?;

        let (archived_at, disabled) = match req.action() {
            CouponAction::Archive => {
                let now = chrono::Utc::now().naive_utc();
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
            .update_coupon_status(patch)
            .await
            .map(|x| CouponWrapper::from(x).0)
            .map_err(Into::<CouponApiError>::into)?;

        Ok(Response::new(UpdateCouponStatusResponse {
            coupon: Some(updated),
        }))
    }
}
