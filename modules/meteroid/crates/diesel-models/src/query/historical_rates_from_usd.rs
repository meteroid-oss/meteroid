use crate::errors::IntoDbResult;
use crate::historical_rates_from_usd::{HistoricalRatesFromUsdRow, HistoricalRatesFromUsdRowNew};

use crate::{DbResult, PgConn};
use diesel::query_dsl::methods::{FilterDsl, LimitDsl, OrderDsl};
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
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting historical_rates_from_usd")
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
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .optional()
            .attach_printable("Error while getting historical_rates_from_usd by date")
            .into_db_result()
    }
}
