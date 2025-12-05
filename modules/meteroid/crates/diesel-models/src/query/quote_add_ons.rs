use crate::errors::IntoDbResult;
use crate::quote_add_ons::{QuoteAddOnRow, QuoteAddOnRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::QuoteId;
use diesel::{ExpressionMethods, QueryDsl, debug_query};
use error_stack::ResultExt;

impl QuoteAddOnRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteAddOnRow> {
        use crate::schema::quote_add_on::dsl::quote_add_on;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote_add_on).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting quote add-on")
            .into_db_result()
    }

    pub async fn insert_batch(
        rows: &[QuoteAddOnRowNew],
        conn: &mut PgConn,
    ) -> DbResult<Vec<QuoteAddOnRow>> {
        use crate::schema::quote_add_on::dsl::quote_add_on;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote_add_on).values(rows);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while batch inserting quote add-ons")
            .into_db_result()
    }
}

impl QuoteAddOnRow {
    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        param_quote_id: QuoteId,
    ) -> DbResult<Vec<QuoteAddOnRow>> {
        use crate::schema::quote_add_on::dsl::{id, quote_add_on, quote_id};
        use diesel_async::RunQueryDsl;

        let query = quote_add_on
            .filter(quote_id.eq(param_quote_id))
            .order(id.asc());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing quote add-ons")
            .into_db_result()
    }

    pub async fn delete_by_quote_id(conn: &mut PgConn, param_quote_id: QuoteId) -> DbResult<()> {
        use crate::schema::quote_add_on::dsl::{quote_add_on, quote_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(quote_add_on.filter(quote_id.eq(param_quote_id)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting quote add-ons")
            .into_db_result()
            .map(|_| ())
    }
}
