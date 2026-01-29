use crate::domain::coupons::{Coupon, CouponFilter, CouponNew, CouponPatch, CouponStatusPatch};
use crate::domain::{AppliedCouponForDisplay, PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{CouponId, TenantId};
use diesel_models::applied_coupons::AppliedCouponForDisplayRow;
use diesel_models::coupon_plans::{CouponPlanRow, CouponPlanRowNew};
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

    async fn list_coupons_by_codes(
        &self,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<Coupon>>;

    async fn list_coupons_by_codes_tx(
        &self,
        conn: &mut diesel_models::PgConn,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<Coupon>>;
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

        // Batch fetch plan_ids for all coupons
        let coupon_ids: Vec<CouponId> = coupons.items.iter().map(|c| c.id).collect();
        let plan_ids_map = CouponPlanRow::list_by_coupon_ids(&mut conn, &coupon_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let items: Vec<Coupon> = coupons
            .items
            .into_iter()
            .map(|row| {
                let id = row.id;
                let mut coupon: Coupon = row.try_into()?;
                coupon.plan_ids = plan_ids_map.get(&id).cloned().unwrap_or_default();
                Ok(coupon)
            })
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

        Ok(PaginatedVec {
            items,
            total_pages: coupons.total_pages,
            total_results: coupons.total_results,
        })
    }

    async fn get_coupon_by_id(&self, tenant_id: TenantId, id: CouponId) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        let row = CouponRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let plan_ids = CouponPlanRow::list_by_coupon_id(&mut conn, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut coupon: Coupon = row.try_into()?;
        coupon.plan_ids = plan_ids;
        Ok(coupon)
    }

    async fn create_coupon(&self, coupon: CouponNew) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        let plan_ids = coupon.plan_ids.clone();
        let coupon_row: CouponRowNew = coupon.try_into()?;
        let coupon_id = coupon_row.id;

        let row = coupon_row
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Insert plan restrictions if any
        if !plan_ids.is_empty() {
            let plan_rows: Vec<CouponPlanRowNew> = plan_ids
                .iter()
                .map(|plan_id| CouponPlanRowNew {
                    coupon_id,
                    plan_id: *plan_id,
                })
                .collect();
            CouponPlanRowNew::insert_batch(&plan_rows, &mut conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }

        let mut coupon: Coupon = row.try_into()?;
        coupon.plan_ids = plan_ids;
        Ok(coupon)
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

        let coupon_id = coupon.id;
        let plan_ids = coupon.plan_ids.clone();
        let coupon_row: CouponRowPatch = coupon.try_into()?;

        let row = coupon_row
            .patch(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Update plan restrictions if provided
        let final_plan_ids = if let Some(new_plan_ids) = plan_ids {
            // Delete existing and insert new
            CouponPlanRow::delete_by_coupon_id(&mut conn, coupon_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            if !new_plan_ids.is_empty() {
                let plan_rows: Vec<CouponPlanRowNew> = new_plan_ids
                    .iter()
                    .map(|plan_id| CouponPlanRowNew {
                        coupon_id,
                        plan_id: *plan_id,
                    })
                    .collect();
                CouponPlanRowNew::insert_batch(&plan_rows, &mut conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            }
            new_plan_ids
        } else {
            // Fetch current plan_ids
            CouponPlanRow::list_by_coupon_id(&mut conn, coupon_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };

        let mut coupon: Coupon = row.try_into()?;
        coupon.plan_ids = final_plan_ids;
        Ok(coupon)
    }

    async fn update_coupon_status(&self, coupon: CouponStatusPatch) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        let coupon_id = coupon.id;
        let coupon_row: CouponStatusRowPatch = coupon.into();

        let row = coupon_row
            .patch(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let plan_ids = CouponPlanRow::list_by_coupon_id(&mut conn, coupon_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut coupon: Coupon = row.try_into()?;
        coupon.plan_ids = plan_ids;
        Ok(coupon)
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
            &coupon_id,
            &tenant_id,
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(PaginatedVec {
            items: coupons
                .items
                .into_iter()
                .map(std::convert::Into::into)
                .collect::<Vec<_>>(),
            total_pages: coupons.total_pages,
            total_results: coupons.total_results,
        })
    }

    async fn list_coupons_by_codes_tx(
        &self,
        conn: &mut diesel_models::PgConn,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<Coupon>> {
        let rows = CouponRow::list_by_codes(conn, tenant_id, codes)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Batch fetch plan_ids for all coupons
        let coupon_ids: Vec<CouponId> = rows.iter().map(|c| c.id).collect();
        let plan_ids_map = CouponPlanRow::list_by_coupon_ids(conn, &coupon_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(|row| {
                let id = row.id;
                let mut coupon: Coupon = row.try_into()?;
                coupon.plan_ids = plan_ids_map.get(&id).cloned().unwrap_or_default();
                Ok(coupon)
            })
            .collect()
    }

    async fn list_coupons_by_codes(
        &self,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<Coupon>> {
        let mut conn = self.get_conn().await?;

        self.list_coupons_by_codes_tx(&mut conn, tenant_id, codes)
            .await
    }
}
