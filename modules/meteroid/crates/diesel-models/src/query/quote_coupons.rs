use crate::errors::IntoDbResult;
use crate::quote_coupons::{QuoteCouponRow, QuoteCouponRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::QuoteId;
use diesel::{ExpressionMethods, QueryDsl, debug_query};
use error_stack::ResultExt;

impl QuoteCouponRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteCouponRow> {
        use crate::schema::quote_coupon::dsl::quote_coupon;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote_coupon).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting quote coupon")
            .into_db_result()
    }

    pub async fn insert_batch(
        rows: &[QuoteCouponRowNew],
        conn: &mut PgConn,
    ) -> DbResult<Vec<QuoteCouponRow>> {
        use crate::schema::quote_coupon::dsl::quote_coupon;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote_coupon).values(rows);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while batch inserting quote coupons")
            .into_db_result()
    }
}

impl QuoteCouponRow {
    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        param_quote_id: QuoteId,
    ) -> DbResult<Vec<QuoteCouponRow>> {
        use crate::schema::quote_coupon::dsl::{id, quote_coupon, quote_id};
        use diesel_async::RunQueryDsl;

        let query = quote_coupon
            .filter(quote_id.eq(param_quote_id))
            .order(id.asc());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing quote coupons")
            .into_db_result()
    }

    pub async fn delete_by_quote_id(conn: &mut PgConn, param_quote_id: QuoteId) -> DbResult<()> {
        use crate::schema::quote_coupon::dsl::{quote_coupon, quote_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(quote_coupon.filter(quote_id.eq(param_quote_id)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting quote coupons")
            .into_db_result()
            .map(|_| ())
    }
}
