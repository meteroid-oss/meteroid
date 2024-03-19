use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::BillingPeriodEnum;
use diesel::sql_types::{Array, Nullable};
use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::plan_version)]
pub struct PlanVersion {
    pub id: Uuid,
    pub is_draft_version: bool,
    pub plan_id: Uuid,
    pub version: i32,
    pub trial_duration_days: Option<i32>,
    pub trial_fallback_plan_id: Option<Uuid>,
    pub tenant_id: Uuid,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub billing_periods: Vec<Option<BillingPeriodEnum>>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::plan_version)]
pub struct PlanVersionNew {
    pub id: Uuid,
    pub is_draft_version: bool,
    pub plan_id: Uuid,
    pub version: i32,
    pub trial_duration_days: Option<i32>,
    pub trial_fallback_plan_id: Option<Uuid>,
    pub tenant_id: Uuid,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    pub created_by: Uuid,
    pub billing_periods: Vec<Option<BillingPeriodEnum>>,
}
