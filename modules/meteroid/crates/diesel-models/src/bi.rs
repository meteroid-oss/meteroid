use chrono::NaiveDate;
use chrono::NaiveDateTime;
use uuid::Uuid;

use super::enums::MrrMovementType;
use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::bi_customer_ytd_summary, primary_key(tenant_id, customer_id, currency, revenue_year))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BiCustomerYtdSummaryRow {
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub revenue_year: i32,
    pub currency: String,
    pub total_revenue_cents: i64,
}

#[derive(Queryable, Debug, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::bi_delta_mrr_daily, primary_key(tenant_id, plan_version_id, currency, date))]
pub struct BiDeltaMrrDailyRow {
    pub tenant_id: Uuid,
    pub plan_version_id: Uuid,
    pub date: NaiveDate,
    pub currency: String,
    pub net_mrr_cents: i64,
    pub new_business_cents: i64,
    pub new_business_count: i32,
    pub expansion_cents: i64,
    pub expansion_count: i32,
    pub contraction_cents: i64,
    pub contraction_count: i32,
    pub churn_cents: i64,
    pub churn_count: i32,
    pub reactivation_cents: i64,
    pub reactivation_count: i32,
    pub historical_rate_id: Uuid,
    pub net_mrr_cents_usd: i64,
    pub new_business_cents_usd: i64,
    pub expansion_cents_usd: i64,
    pub contraction_cents_usd: i64,
    pub churn_cents_usd: i64,
    pub reactivation_cents_usd: i64,
}

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::bi_mrr_movement_log)]
pub struct BiMrrMovementLogRow {
    pub id: Uuid,
    pub description: String,
    pub movement_type: MrrMovementType,
    pub net_mrr_change: i64,
    pub currency: String,
    pub created_at: NaiveDateTime,
    pub applies_to: NaiveDate,
    pub invoice_id: Uuid,
    pub credit_note_id: Option<Uuid>,
    pub plan_version_id: Uuid,
    pub tenant_id: Uuid,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::bi_mrr_movement_log)]
pub struct BiMrrMovementLogRowNew {
    pub id: Uuid,
    pub description: String,
    pub movement_type: MrrMovementType,
    pub net_mrr_change: i64,
    pub currency: String,
    pub applies_to: NaiveDate,
    pub invoice_id: Uuid,
    pub credit_note_id: Option<Uuid>,
    pub plan_version_id: Uuid,
    pub tenant_id: Uuid,
}

#[derive(Queryable, Debug, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::bi_revenue_daily)]
#[diesel(primary_key(tenant_id, plan_version_id, currency, revenue_date))]
pub struct BiRevenueDailyRow {
    pub tenant_id: Uuid,
    pub plan_version_id: Option<Uuid>,
    pub currency: String,
    pub revenue_date: NaiveDate,
    pub net_revenue_cents: i64,
    pub historical_rate_id: Uuid,
    pub net_revenue_cents_usd: i64,
}
