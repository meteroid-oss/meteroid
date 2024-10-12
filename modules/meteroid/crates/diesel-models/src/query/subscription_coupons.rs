use crate::errors::IntoDbResult;
use crate::subscription_coupons::{SubscriptionCouponRow, SubscriptionCouponRowNew};
use crate::{DbResult, PgConn};
use diesel::debug_query;
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
}
