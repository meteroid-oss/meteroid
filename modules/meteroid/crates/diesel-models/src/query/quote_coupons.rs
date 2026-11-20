use crate::errors::IntoDbResult;
use crate::quote_coupons::{QuoteCouponRow, QuoteCouponRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{CouponId, QuoteId, TenantId};
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl QuoteCouponRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteCouponRow> {
        use crate::schema::quote_coupon::dsl::quote_coupon;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote_coupon).values(self);

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing quote coupons")
            .into_db_result()
    }

    /// Numbers of the quotes offering the coupon that can still become a subscription with it:
    /// pending, or accepted and not converted yet.
    pub async fn list_open_quote_numbers_by_coupon_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        coupon_id: CouponId,
        limit: i64,
    ) -> DbResult<Vec<String>> {
        use crate::enums::QuoteStatusEnum;
        use crate::schema::quote::dsl as q;
        use crate::schema::quote_coupon::dsl as qc;
        use diesel_async::RunQueryDsl;

        let query = qc::quote_coupon
            .inner_join(q::quote)
            .filter(qc::coupon_id.eq(coupon_id))
            .filter(q::tenant_id.eq(tenant_id))
            .filter(
                q::status.eq(QuoteStatusEnum::Pending).or(q::status
                    .eq(QuoteStatusEnum::Accepted)
                    .and(q::converted_to_subscription_id.is_null())),
            )
            .select(q::quote_number)
            .distinct()
            .order(q::quote_number.asc())
            .limit(limit);

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing the open quotes of a coupon")
            .into_db_result()
    }

    pub async fn delete_by_quote_id(conn: &mut PgConn, param_quote_id: QuoteId) -> DbResult<()> {
        use crate::schema::quote_coupon::dsl::{quote_coupon, quote_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(quote_coupon.filter(quote_id.eq(param_quote_id)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting quote coupons")
            .into_db_result()
            .map(|_| ())
    }
}
