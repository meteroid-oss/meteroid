use crate::api::coupons::error::CouponApiError;
use crate::api::coupons::mapping::coupons::CouponWrapper;
use crate::api::coupons::{mapping, CouponsServiceComponents};
use crate::api::shared::mapping::datetime::chrono_from_timestamp;
use crate::{api::utils::parse_uuid, parse_uuid};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::coupons::v1::coupons_service_server::CouponsService;
use meteroid_grpc::meteroid::api::coupons::v1::{
    CreateCouponRequest, CreateCouponResponse, EditCouponRequest, EditCouponResponse,
    ListCouponRequest, ListCouponResponse, RemoveCouponRequest, RemoveCouponResponse,
};
use meteroid_store::domain;
use meteroid_store::repositories::coupons::CouponInterface;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl CouponsService for CouponsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_coupons(
        &self,
        request: Request<ListCouponRequest>,
    ) -> Result<Response<ListCouponResponse>, Status> {
        let tenant_id = request.tenant()?;

        let coupons = self
            .store
            .list_coupons(tenant_id)
            .await
            .map_err(Into::<CouponApiError>::into)?
            .into_iter()
            .map(|x| CouponWrapper::from(x).0)
            .collect();

        let response = ListCouponResponse { coupons };

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
            expires_at: req.expires_at.map(chrono_from_timestamp).transpose()?,
            redemption_limit: req.redemption_limit,
            tenant_id,
            recurring_value: None, // todo fixme later
            reusable: false,       // todo fixme later
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

        let id = parse_uuid!(&req.coupon_id)?;

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
            id: parse_uuid!(&req.coupon_id)?,
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
}
