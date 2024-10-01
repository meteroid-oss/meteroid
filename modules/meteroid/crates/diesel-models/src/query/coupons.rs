use crate::coupons::{CouponRow, CouponRowNew, CouponRowPatch};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use diesel::{debug_query, ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use tap::TapFallible;

impl CouponRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::insert_into(c_dsl::coupon).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

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
        tenant_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting coupon")
            .into_db_result()
    }

    pub async fn list_by_tenant_id(
        conn: &mut PgConn,
        tenant_id: uuid::Uuid,
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon.filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing coupons")
            .into_db_result()
    }

    pub async fn delete(
        conn: &mut PgConn,
        tenant_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> DbResult<usize> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::delete(c_dsl::coupon)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting coupon")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[uuid::Uuid],
        tenant_id: &uuid::Uuid,
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon
            .filter(c_dsl::id.eq_any(ids))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching coupons: {:?}", e))
            .attach_printable("Error while fetching coupons")
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating coupon")
            .into_db_result()
    }
}
