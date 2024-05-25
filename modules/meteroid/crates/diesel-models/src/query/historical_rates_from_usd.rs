use crate::errors::IntoDbResult;
use crate::historical_rates_from_usd::{HistoricalRatesFromUsd, HistoricalRatesFromUsdNew};

use crate::{DbResult, PgConn};
use diesel::{debug_query, ExpressionMethods};
use error_stack::ResultExt;

impl HistoricalRatesFromUsdNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<HistoricalRatesFromUsd> {
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
