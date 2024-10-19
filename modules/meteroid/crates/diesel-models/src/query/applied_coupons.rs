use crate::applied_coupons::{AppliedCouponRow, AppliedCouponRowNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use diesel::{debug_query, ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

impl AppliedCouponRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<AppliedCouponRow> {
        use crate::schema::applied_coupon::dsl as ac;

        let query = diesel::insert_into(ac::applied_coupon).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting applied coupon")
            .into_db_result()
    }
}

impl AppliedCouponRow {
    pub async fn count_by_coupon_id(conn: &mut PgConn, param_coupon_id: &Uuid) -> DbResult<i64> {
        use crate::schema::applied_coupon::dsl as ac;

        let query = ac::applied_coupon
            .filter(ac::coupon_id.eq(param_coupon_id))
            .count();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while counting applied coupons by coupon id")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: Vec<&AppliedCouponRowNew>,
    ) -> DbResult<Vec<AppliedCouponRow>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;

        let query = diesel::insert_into(ac_dsl::applied_coupon).values(batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting AppliedCoupon batch")
            .into_db_result()
    }
}
