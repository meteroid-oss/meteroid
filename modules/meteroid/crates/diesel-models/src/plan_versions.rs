use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::ActionAfterTrialEnum;

use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionRow {
    pub id: Uuid,
    pub is_draft_version: bool,
    pub plan_id: Uuid,
    pub version: i32,
    pub trial_duration_days: Option<i32>,
    pub downgrade_plan_id: Option<Uuid>,
    pub tenant_id: Uuid,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub trialing_plan_id: Option<Uuid>,
    pub action_after_trial: Option<ActionAfterTrialEnum>,
    pub trial_is_free: bool,
}

#[derive(Debug, Insertable, Default)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionRowNew {
    pub id: Uuid,
    pub is_draft_version: bool,
    pub plan_id: Uuid,
    pub version: i32,
    pub trial_duration_days: Option<i32>,
    pub downgrade_plan_id: Option<Uuid>,
    pub tenant_id: Uuid,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    pub created_by: Uuid,
    pub trialing_plan_id: Option<Uuid>,
    pub action_after_trial: Option<ActionAfterTrialEnum>,
    pub trial_is_free: bool,
}

#[derive(Debug, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionRowLatest {
    pub id: Uuid,
    pub plan_id: Uuid,
    #[diesel(select_expression = crate::schema::plan::name)]
    #[diesel(select_expression_type = crate::schema::plan::name)]
    pub plan_name: String,
    #[diesel(select_expression = crate::schema::plan::local_id)]
    #[diesel(select_expression_type = crate::schema::plan::local_id)]
    pub local_id: String,
    pub version: i32,
    pub created_by: Uuid,
    pub trial_duration_days: Option<i32>,
    pub downgrade_plan_id: Option<Uuid>,
    pub trialing_plan_id: Option<Uuid>,
    pub action_after_trial: Option<ActionAfterTrialEnum>,
    pub trial_is_free: bool,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    #[diesel(select_expression = crate::schema::product_family::id)]
    #[diesel(select_expression_type = crate::schema::product_family::id)]
    pub product_family_id: Uuid,
    #[diesel(select_expression = crate::schema::product_family::name)]
    #[diesel(select_expression_type = crate::schema::product_family::name)]
    pub product_family_name: String,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct PlanVersionRowPatch {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub currency: Option<String>,
    pub net_terms: Option<i32>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct PlanVersionTrialRowPatch {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub trialing_plan_id: Option<Option<Uuid>>,
    pub action_after_trial: Option<Option<ActionAfterTrialEnum>>,
    pub trial_is_free: Option<bool>,
    pub trial_duration_days: Option<Option<i32>>,
    pub downgrade_plan_id: Option<Option<Uuid>>,
}
