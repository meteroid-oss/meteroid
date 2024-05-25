use chrono::NaiveDate;
use diesel::{Identifiable, Insertable, Queryable};
use uuid::Uuid;

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::historical_rates_from_usd)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct HistoricalRatesFromUsdNew {
    pub id: Uuid,
    pub date: NaiveDate,
    pub rates: serde_json::Value,
}

#[derive(Debug, Queryable, Identifiable)]
#[diesel(table_name = crate::schema::historical_rates_from_usd)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct HistoricalRatesFromUsd {
    pub id: Uuid,
    pub date: NaiveDate,
    pub rates: serde_json::Value,
}
