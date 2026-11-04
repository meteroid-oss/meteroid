use diesel::QueryableByName;

/// Pre-tax revenue of production tenants finalized on one UTC day, in one currency.
#[derive(Debug, QueryableByName)]
pub struct DailyRevenueRow {
    #[diesel(sql_type = diesel::sql_types::Date)]
    pub day: chrono::NaiveDate,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub currency: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub amount_cents: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub invoice_count: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub credit_note_count: i64,
}
