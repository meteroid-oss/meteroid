use crate::errors::IntoDbResult;
use crate::historical_rates_from_usd::{HistoricalRatesFromUsdRow, HistoricalRatesFromUsdRowNew};

use crate::{DbResult, PgConn};
use diesel::query_dsl::methods::{FilterDsl, LimitDsl, OrderDsl};
use diesel::upsert::excluded;
use diesel::{ExpressionMethods, OptionalExtension};
use error_stack::ResultExt;

impl HistoricalRatesFromUsdRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<HistoricalRatesFromUsdRow> {
        use crate::schema::historical_rates_from_usd::dsl as r_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(r_dsl::historical_rates_from_usd)
            .values(self)
            .on_conflict(r_dsl::date)
            .do_update()
            .set(r_dsl::rates.eq(&self.rates));
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting historical_rates_from_usd")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        values: Vec<HistoricalRatesFromUsdRowNew>,
    ) -> DbResult<()> {
        use crate::schema::historical_rates_from_usd::dsl as r_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(r_dsl::historical_rates_from_usd)
            .values(values)
            .on_conflict(r_dsl::date)
            .do_update()
            .set(r_dsl::rates.eq(excluded(r_dsl::rates)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(drop)
            .attach("Error while inserting batch historical_rates_from_usd")
            .into_db_result()
    }
}

impl HistoricalRatesFromUsdRow {
    pub async fn get_by_date(
        date: chrono::NaiveDate,
        conn: &mut PgConn,
    ) -> DbResult<Option<HistoricalRatesFromUsdRow>> {
        use crate::schema::historical_rates_from_usd::dsl as r_dsl;
        use diesel_async::RunQueryDsl;

        let query = r_dsl::historical_rates_from_usd
            .filter(r_dsl::date.le(date))
            .order(r_dsl::date.desc())
            .limit(1);
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while getting historical_rates_from_usd by date")
            .into_db_result()
    }

    /// The rows in force on each of `dates` (latest on or before it), each once.
    pub async fn list_in_force_on(
        dates: &[chrono::NaiveDate],
        conn: &mut PgConn,
    ) -> DbResult<Vec<HistoricalRatesFromUsdRow>> {
        use diesel_async::RunQueryDsl;

        diesel::sql_query(
            r"
            SELECT DISTINCT ON (r.id) r.id, r.date, r.rates, r.updated_at
            FROM unnest($1::date[]) AS d(day)
            CROSS JOIN LATERAL (
                SELECT * FROM historical_rates_from_usd h
                WHERE h.date <= d.day
                ORDER BY h.date DESC
                LIMIT 1
            ) r
            ",
        )
        .bind::<diesel::sql_types::Array<diesel::sql_types::Date>, _>(dates)
        .get_results(conn)
        .await
        .attach("Error while listing historical_rates_from_usd in force on dates")
        .into_db_result()
    }

    pub async fn latest(conn: &mut PgConn) -> DbResult<Option<HistoricalRatesFromUsdRow>> {
        use crate::schema::historical_rates_from_usd::dsl as r_dsl;
        use diesel_async::RunQueryDsl;

        let query = r_dsl::historical_rates_from_usd.order(r_dsl::date.desc());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while getting latest historical_rates_from_usd ")
            .into_db_result()
    }
}
