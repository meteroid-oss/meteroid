use crate::errors::IntoDbResult;
use crate::historical_rates_from_usd::{HistoricalRatesFromUsdRow, HistoricalRatesFromUsdRowNew};

use crate::{DbResult, PgConn};
use diesel::query_dsl::methods::{FilterDsl, LimitDsl, OrderDsl};
use diesel::upsert::excluded;
use diesel::{ExpressionMethods, OptionalExtension, debug_query};
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
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while getting historical_rates_from_usd by date")
            .into_db_result()
    }

    pub async fn latest(conn: &mut PgConn) -> DbResult<Option<HistoricalRatesFromUsdRow>> {
        use crate::schema::historical_rates_from_usd::dsl as r_dsl;
        use diesel_async::RunQueryDsl;

        let query = r_dsl::historical_rates_from_usd.order(r_dsl::date.desc());
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while getting latest historical_rates_from_usd ")
            .into_db_result()
    }
}
