use chrono::{NaiveDate, NaiveDateTime};
use diesel::{Identifiable, Insertable, Queryable};
use uuid::Uuid;

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::historical_rates_from_usd)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct HistoricalRatesFromUsdRowNew {
    pub id: Uuid,
    pub date: NaiveDate,
    pub rates: serde_json::Value,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Queryable, Identifiable)]
#[diesel(table_name = crate::schema::historical_rates_from_usd)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct HistoricalRatesFromUsdRow {
    pub id: Uuid,
    pub date: NaiveDate,
    pub rates: serde_json::Value,
    pub updated_at: NaiveDateTime,
}
