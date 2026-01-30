use crate::coupon_plans::{CouponPlanRow, CouponPlanRowNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{CouponId, PlanId};
use diesel::{ExpressionMethods, QueryDsl, debug_query};
use error_stack::ResultExt;
use std::collections::HashMap;

impl CouponPlanRowNew {
    pub async fn insert_batch(rows: &[CouponPlanRowNew], conn: &mut PgConn) -> DbResult<usize> {
        use crate::schema::coupon_plan::dsl::coupon_plan;
        use diesel_async::RunQueryDsl;

        if rows.is_empty() {
            return Ok(0);
        }

        let query = diesel::insert_into(coupon_plan).values(rows);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while batch inserting coupon plans")
            .into_db_result()
    }
}

impl CouponPlanRow {
    pub async fn list_by_coupon_id(
        conn: &mut PgConn,
        param_coupon_id: CouponId,
    ) -> DbResult<Vec<PlanId>> {
        use crate::schema::coupon_plan::dsl::{coupon_id, coupon_plan, plan_id};
        use diesel_async::RunQueryDsl;

        let query = coupon_plan
            .filter(coupon_id.eq(param_coupon_id))
            .select(plan_id);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing coupon plans")
            .into_db_result()
    }

    pub async fn list_by_coupon_ids(
        conn: &mut PgConn,
        coupon_ids: &[CouponId],
    ) -> DbResult<HashMap<CouponId, Vec<PlanId>>> {
        use crate::schema::coupon_plan::dsl::{coupon_id, coupon_plan};
        use diesel_async::RunQueryDsl;

        if coupon_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let query = coupon_plan.filter(coupon_id.eq_any(coupon_ids));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let rows: Vec<CouponPlanRow> = query
            .load(conn)
            .await
            .attach("Error while listing coupon plans by ids")
            .into_db_result()?;

        let mut result: HashMap<CouponId, Vec<PlanId>> = HashMap::new();
        for row in rows {
            result.entry(row.coupon_id).or_default().push(row.plan_id);
        }

        Ok(result)
    }

    pub async fn delete_by_coupon_id(conn: &mut PgConn, param_coupon_id: CouponId) -> DbResult<()> {
        use crate::schema::coupon_plan::dsl::{coupon_id, coupon_plan};
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(coupon_plan.filter(coupon_id.eq(param_coupon_id)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting coupon plans")
            .into_db_result()
            .map(|_| ())
    }
}
