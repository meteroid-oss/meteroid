use crate::coupons::{CouponFilter, CouponRow, CouponRowNew, CouponRowPatch, CouponStatusRowPatch};
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};

use crate::{DbResult, PgConn};
use common_domain::ids::{CouponId, TenantId};
use diesel::dsl::not;
use diesel::{
    BoolExpressionMethods, ExpressionMethods, NullableExpressionMethods, PgTextExpressionMethods,
    QueryDsl, debug_query,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use std::collections::HashMap;
use tap::TapFallible;

impl CouponRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::insert_into(c_dsl::coupon).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting coupon")
            .into_db_result()
    }
}

impl CouponRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CouponId,
    ) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while getting coupon")
            .into_db_result()
    }

    pub async fn list_by_tenant_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        search: Option<String>,
        filter: CouponFilter,
    ) -> DbResult<PaginatedVec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        let mut query = c_dsl::coupon
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if let Some(search) = search {
            query = query.filter(c_dsl::code.ilike(format!("%{}%", search)));
        }

        let is_expired = c_dsl::expires_at
            .is_not_null()
            .and(c_dsl::expires_at.lt(chrono::Utc::now().naive_utc()));

        let is_exhausted = c_dsl::redemption_limit
            .is_not_null()
            .and(c_dsl::redemption_limit.le(c_dsl::redemption_count.nullable()));

        let is_archived = c_dsl::archived_at.is_not_null();

        let is_disabled = c_dsl::disabled.eq(true);

        match filter {
            CouponFilter::ACTIVE => {
                query = query.filter(
                    not(is_expired)
                        .and(not(is_exhausted))
                        .and(not(is_archived))
                        .and(not(is_disabled)),
                );
            }
            CouponFilter::ARCHIVED => {
                query = query.filter(is_archived);
            }
            CouponFilter::INACTIVE => {
                query = query.filter(is_disabled.or(is_expired).or(is_exhausted));
            }
            CouponFilter::ALL => {}
        }

        let query = query.order(c_dsl::created_at.desc()).paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while listing coupons")
            .into_db_result()
    }

    pub async fn delete(conn: &mut PgConn, tenant_id: TenantId, id: CouponId) -> DbResult<usize> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::delete(c_dsl::coupon)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting coupon")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[CouponId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon
            .filter(c_dsl::id.eq_any(ids))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching coupons: {:?}", e))
            .attach_printable("Error while fetching coupons")
            .into_db_result()
    }

    pub async fn list_by_ids_for_update(
        conn: &mut PgConn,
        ids: &[CouponId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon
            .filter(c_dsl::id.eq_any(ids))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .for_update();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching coupons for update")
            .into_db_result()
    }

    pub async fn inc_redemption_count(
        conn: &mut PgConn,
        coupon_id: CouponId,
        delta: i32,
    ) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::update(c_dsl::coupon)
            .filter(c_dsl::id.eq(coupon_id))
            .set(c_dsl::redemption_count.eq(c_dsl::redemption_count + delta));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while incrementing coupon redemption count")
            .into_db_result()
    }

    pub async fn customers_count(
        conn: &mut PgConn,
        coupons: &[CouponId],
    ) -> DbResult<HashMap<CouponId, i64>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;

        let query = ac_dsl::applied_coupon
            .filter(ac_dsl::coupon_id.eq_any(coupons))
            .group_by(ac_dsl::coupon_id)
            .select((ac_dsl::coupon_id, diesel::dsl::count(ac_dsl::customer_id)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load::<(CouponId, i64)>(conn)
            .await
            .attach_printable("Error while counting customers for coupons")
            .into_db_result()
            .map(|rows: Vec<(CouponId, i64)>| rows.into_iter().collect())
    }

    pub async fn update_last_redemption_at(
        conn: &mut PgConn,
        coupon_ids: &[CouponId],
        last_redemption_at: chrono::NaiveDateTime,
    ) -> DbResult<()> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::update(c_dsl::coupon)
            .filter(c_dsl::id.eq_any(coupon_ids))
            .set(c_dsl::last_redemption_at.eq(last_redemption_at));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach_printable("Error while updating coupon last redemption at")
            .into_db_result()
    }

    pub async fn list_by_codes(
        conn: &mut PgConn,
        tenant_id: TenantId,
        codes: &[String],
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::code.eq_any(codes));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach_printable("Error while listing coupons by codes")
            .into_db_result()
    }
}

impl CouponRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::update(c_dsl::coupon)
            .filter(c_dsl::id.eq(self.id))
            .filter(c_dsl::tenant_id.eq(self.tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating coupon")
            .into_db_result()
    }
}

impl CouponStatusRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::update(c_dsl::coupon)
            .filter(c_dsl::id.eq(self.id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating coupon status")
            .into_db_result()
    }
}
