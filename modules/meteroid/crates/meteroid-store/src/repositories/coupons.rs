use crate::domain::coupons::{Coupon, CouponNew, CouponPatch};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use diesel_models::coupons::{CouponRow, CouponRowNew, CouponRowPatch};
use error_stack::Report;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait CouponInterface {
    async fn list_coupons(&self, tenant_id: Uuid) -> StoreResult<Vec<Coupon>>;
    async fn get_coupon_by_id(&self, tenant_id: Uuid, id: Uuid) -> StoreResult<Coupon>;
    async fn create_coupon(&self, coupon: CouponNew) -> StoreResult<Coupon>;
    async fn delete_coupon(&self, tenant_id: Uuid, id: Uuid) -> StoreResult<()>;
    async fn update_coupon(&self, coupon: CouponPatch) -> StoreResult<Coupon>;
}

#[async_trait::async_trait]
impl CouponInterface for Store {
    async fn list_coupons(&self, tenant_id: Uuid) -> StoreResult<Vec<Coupon>> {
        let mut conn = self.get_conn().await?;

        let coupons = CouponRow::list_by_tenant_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        coupons
            .into_iter()
            .map(|s| s.try_into())
            .collect::<Result<Vec<_>, _>>()
    }

    async fn get_coupon_by_id(&self, tenant_id: Uuid, id: Uuid) -> StoreResult<Coupon> {
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

    async fn delete_coupon(&self, tenant_id: Uuid, id: Uuid) -> StoreResult<()> {
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
}
