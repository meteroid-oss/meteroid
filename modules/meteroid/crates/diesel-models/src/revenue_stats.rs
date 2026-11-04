use chrono::{NaiveDate, NaiveDateTime};
use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Date, Integer, Nullable, Text, Timestamp, Uuid as SqlUuid};
use uuid::Uuid;

/// MRR movements of one day, currency and (when split) plan, in the movement currency.
#[derive(QueryableByName, Debug)]
pub struct MrrMovementDayRow {
    #[diesel(sql_type = Date)]
    pub date: NaiveDate,
    #[diesel(sql_type = Text)]
    pub currency: String,
    #[diesel(sql_type = Nullable<SqlUuid>)]
    pub plan_id: Option<Uuid>,
    #[diesel(sql_type = BigInt)]
    pub net: i64,
    #[diesel(sql_type = BigInt)]
    pub new_business: i64,
    #[diesel(sql_type = Integer)]
    pub new_business_count: i32,
    #[diesel(sql_type = BigInt)]
    pub expansion: i64,
    #[diesel(sql_type = Integer)]
    pub expansion_count: i32,
    #[diesel(sql_type = BigInt)]
    pub contraction: i64,
    #[diesel(sql_type = Integer)]
    pub contraction_count: i32,
    #[diesel(sql_type = BigInt)]
    pub churn: i64,
    #[diesel(sql_type = Integer)]
    pub churn_count: i32,
    #[diesel(sql_type = BigInt)]
    pub reactivation: i64,
    #[diesel(sql_type = Integer)]
    pub reactivation_count: i32,
}

#[derive(QueryableByName, Debug)]
pub struct MrrLogRow {
    #[diesel(sql_type = Timestamp)]
    pub created_at: NaiveDateTime,
    #[diesel(sql_type = Date)]
    pub applies_to: NaiveDate,
    #[diesel(sql_type = SqlUuid)]
    pub customer_id: Uuid,
    #[diesel(sql_type = Text)]
    pub customer_name: String,
    #[diesel(sql_type = SqlUuid)]
    pub subscription_id: Uuid,
    #[diesel(sql_type = Text)]
    pub plan_name: String,
    #[diesel(sql_type = Text)]
    pub description: String,
    #[diesel(sql_type = crate::schema::sql_types::MrrMovementType)]
    pub movement_type: crate::enums::MrrMovementType,
    #[diesel(sql_type = BigInt)]
    pub net_mrr_change: i64,
    #[diesel(sql_type = Text)]
    pub currency: String,
}

/// Billed net revenue of one day and currency (and customer, for the ranking).
#[derive(QueryableByName, Debug)]
pub struct RevenueDayRow {
    #[diesel(sql_type = Date)]
    pub date: NaiveDate,
    #[diesel(sql_type = Text)]
    pub currency: String,
    #[diesel(sql_type = Nullable<SqlUuid>)]
    pub customer_id: Option<Uuid>,
    #[diesel(sql_type = BigInt)]
    pub net: i64,
}

#[derive(QueryableByName, Debug)]
pub struct CustomerNameRow {
    #[diesel(sql_type = SqlUuid)]
    pub id: Uuid,
    #[diesel(sql_type = Text)]
    pub name: String,
}
