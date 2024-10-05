use crate::coupons::{CouponRow, CouponRowNew, CouponRowPatch};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use diesel::{debug_query, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use std::collections::HashMap;
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

    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        tenant_id: &uuid::Uuid,
        subscription_id: &uuid::Uuid,
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_coupon::dsl as sc_dsl;

        let query = sc_dsl::subscription_coupon
            .inner_join(s_dsl::subscription)
            .inner_join(c_dsl::coupon)
            .filter(sc_dsl::subscription_id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .select(CouponRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing Coupon by subscription_id")
            .into_db_result()
    }

    pub async fn list_by_ids_for_update(
        conn: &mut PgConn,
        ids: &[uuid::Uuid],
        tenant_id: &uuid::Uuid,
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon
            .filter(c_dsl::id.eq_any(ids))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .for_update();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching coupons for update")
            .into_db_result()
    }

    pub async fn subscriptions_count(
        conn: &mut PgConn,
        coupons: &[uuid::Uuid],
    ) -> DbResult<HashMap<uuid::Uuid, i64>> {
        use crate::schema::subscription_coupon::dsl as sc_dsl;

        let query = sc_dsl::subscription_coupon
            .filter(sc_dsl::coupon_id.eq_any(coupons))
            .group_by(sc_dsl::coupon_id)
            .select((
                sc_dsl::coupon_id,
                diesel::dsl::count(sc_dsl::subscription_id),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .load::<(uuid::Uuid, i64)>(conn)
            .await
            .attach_printable("Error while counting subscriptions for coupons")
            .into_db_result()
            .map(|rows: Vec<(uuid::Uuid, i64)>| rows.into_iter().collect())
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
