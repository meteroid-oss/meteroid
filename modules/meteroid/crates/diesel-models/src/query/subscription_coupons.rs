use crate::errors::IntoDbResult;
use crate::subscription_coupons::{SubscriptionCouponRow, SubscriptionCouponRowNew};
use crate::{DbResult, PgConn};
use diesel::{debug_query, QueryDsl};
use diesel::{ExpressionMethods, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl SubscriptionCouponRow {
    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionCouponRowNew>,
    ) -> DbResult<Vec<SubscriptionCouponRow>> {
        use crate::schema::subscription_coupon::dsl as sc_dsl;

        let query = diesel::insert_into(sc_dsl::subscription_coupon).values(batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting SubscriptionCoupon batch")
            .into_db_result()
    }

    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        tenant_id: &uuid::Uuid,
        subscription_id: &uuid::Uuid,
    ) -> DbResult<Vec<SubscriptionCouponRow>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_coupon::dsl as sc_dsl;

        let query = sc_dsl::subscription_coupon
            .inner_join(s_dsl::subscription)
            .filter(sc_dsl::subscription_id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .select(SubscriptionCouponRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing SubscriptionCoupon by subscription_id")
            .into_db_result()
    }
}
