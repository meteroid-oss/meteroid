use crate::applied_coupons::{AppliedCouponDetailedRow, AppliedCouponRow, AppliedCouponRowNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use diesel::{debug_query, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use rust_decimal::Decimal;
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

    pub async fn refresh_state(
        conn: &mut PgConn,
        id: Uuid,
        amount_delta: Option<Decimal>,
    ) -> DbResult<AppliedCouponRow> {
        use crate::schema::applied_coupon::dsl as ac_dsl;

        let now = chrono::Utc::now().naive_utc();
        let amount_delta = amount_delta.unwrap_or(Decimal::ZERO);

        let query = diesel::update(ac_dsl::applied_coupon)
            .filter(ac_dsl::id.eq(id))
            .set((
                ac_dsl::last_applied_at.eq(now),
                ac_dsl::applied_count.eq(ac_dsl::applied_count + 1),
                ac_dsl::applied_amount.eq(ac_dsl::applied_amount + amount_delta),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while finalizing invoice")
            .into_db_result()
    }
}

impl AppliedCouponDetailedRow {
    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        param_subscription_id: &Uuid,
    ) -> DbResult<Vec<AppliedCouponDetailedRow>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;
        use crate::schema::coupon::dsl as c_dsl;

        let query = ac_dsl::applied_coupon
            .inner_join(c_dsl::coupon)
            .filter(ac_dsl::subscription_id.eq(param_subscription_id))
            .select(AppliedCouponDetailedRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing applied coupons by subscription id")
            .into_db_result()
    }

    pub async fn list_by_ids_for_update(
        conn: &mut PgConn,
        applied_coupons_ids: &[Uuid],
    ) -> DbResult<Vec<AppliedCouponDetailedRow>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;
        use crate::schema::coupon::dsl as c_dsl;

        let query = ac_dsl::applied_coupon
            .inner_join(c_dsl::coupon)
            .filter(ac_dsl::id.eq_any(applied_coupons_ids))
            .select(AppliedCouponDetailedRow::as_select())
            .for_update();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing applied coupons by applied_coupon_id")
            .into_db_result()
    }
}
