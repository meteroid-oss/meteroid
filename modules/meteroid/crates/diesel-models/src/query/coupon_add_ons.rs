use crate::coupon_add_ons::{CouponAddOnRow, CouponAddOnRowNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{AddOnId, CouponId};
use diesel::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;
use std::collections::HashMap;

impl CouponAddOnRowNew {
    pub async fn insert_batch(rows: &[CouponAddOnRowNew], conn: &mut PgConn) -> DbResult<usize> {
        use crate::schema::coupon_add_on::dsl::coupon_add_on;
        use diesel_async::RunQueryDsl;

        if rows.is_empty() {
            return Ok(0);
        }

        let query = diesel::insert_into(coupon_add_on).values(rows);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while batch inserting coupon add-ons")
            .into_db_result()
    }
}

impl CouponAddOnRow {
    pub async fn list_by_coupon_ids(
        conn: &mut PgConn,
        coupon_ids: &[CouponId],
    ) -> DbResult<HashMap<CouponId, Vec<AddOnId>>> {
        use crate::schema::coupon_add_on::dsl::{coupon_add_on, coupon_id};
        use diesel_async::RunQueryDsl;

        if coupon_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let query = coupon_add_on.filter(coupon_id.eq_any(coupon_ids));

        crate::debug_sql(&query);

        let rows: Vec<CouponAddOnRow> = query
            .load(conn)
            .await
            .attach("Error while listing coupon add-ons by ids")
            .into_db_result()?;

        let mut result: HashMap<CouponId, Vec<AddOnId>> = HashMap::new();
        for row in rows {
            result.entry(row.coupon_id).or_default().push(row.add_on_id);
        }

        Ok(result)
    }

    pub async fn delete_by_coupon_id(conn: &mut PgConn, param_coupon_id: CouponId) -> DbResult<()> {
        use crate::schema::coupon_add_on::dsl::{coupon_add_on, coupon_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(coupon_add_on.filter(coupon_id.eq(param_coupon_id)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting coupon add-ons")
            .into_db_result()
            .map(|_| ())
    }
}
