use crate::domain::coupons::{Coupon, CouponFilter, CouponNew, CouponPatch, CouponStatusPatch};
use crate::domain::{AppliedCouponForDisplay, PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{CouponId, TenantId};
use diesel_models::applied_coupons::AppliedCouponForDisplayRow;
use diesel_models::coupons::{CouponRow, CouponRowNew, CouponRowPatch, CouponStatusRowPatch};
use error_stack::Report;

#[async_trait::async_trait]
pub trait CouponInterface {
    async fn list_coupons(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        search: Option<String>,
        filter: CouponFilter,
    ) -> StoreResult<PaginatedVec<Coupon>>;
    async fn get_coupon_by_id(&self, tenant_id: TenantId, id: CouponId) -> StoreResult<Coupon>;

    async fn create_coupon(&self, coupon: CouponNew) -> StoreResult<Coupon>;
    async fn delete_coupon(&self, tenant_id: TenantId, id: CouponId) -> StoreResult<()>;
    async fn update_coupon(&self, coupon: CouponPatch) -> StoreResult<Coupon>;
    async fn update_coupon_status(&self, coupon: CouponStatusPatch) -> StoreResult<Coupon>;

    async fn list_applied_coupons_by_coupon_id(
        &self,
        tenant_id: TenantId,
        coupon_id: CouponId,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<AppliedCouponForDisplay>>;
}

#[async_trait::async_trait]
impl CouponInterface for Store {
    async fn list_coupons(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        search: Option<String>,
        filter: CouponFilter,
    ) -> StoreResult<PaginatedVec<Coupon>> {
        let mut conn = self.get_conn().await?;

        let coupons = CouponRow::list_by_tenant_id(
            &mut conn,
            tenant_id,
            pagination.into(),
            search,
            filter.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(PaginatedVec {
            items: coupons
                .items
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            total_pages: coupons.total_pages,
            total_results: coupons.total_results,
        })
    }

    async fn get_coupon_by_id(&self, tenant_id: TenantId, id: CouponId) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        CouponRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn create_coupon(&self, coupon: CouponNew) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        let coupon: CouponRowNew = coupon.try_into()?;

        coupon
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn delete_coupon(&self, tenant_id: TenantId, id: CouponId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        CouponRow::delete(&mut conn, tenant_id, id)
            .await
            .map_err(Into::into)
            .map(|_| ())
    }

    async fn update_coupon(&self, coupon: CouponPatch) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        let coupon: CouponRowPatch = coupon.try_into()?;

        coupon
            .patch(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)
    }

    async fn update_coupon_status(&self, coupon: CouponStatusPatch) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        let coupon: CouponStatusRowPatch = coupon.into();

        coupon
            .patch(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)
    }

    async fn list_applied_coupons_by_coupon_id(
        &self,
        tenant_id: TenantId,
        coupon_id: CouponId,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<AppliedCouponForDisplay>> {
        let mut conn = self.get_conn().await?;

        let coupons = AppliedCouponForDisplayRow::list_by_coupon_id(
            &mut conn,
            coupon_id,
            tenant_id,
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(PaginatedVec {
            items: coupons
                .items
                .into_iter()
                .map(|s| s.into())
                .collect::<Vec<_>>(),
            total_pages: coupons.total_pages,
            total_results: coupons.total_results,
        })
    }
}
