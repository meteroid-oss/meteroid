use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;
// TODO duplicate as well
use diesel_models::enums::{BillingPeriodEnum, PlanStatusEnum, PlanTypeEnum};

use o2o::traits::IntoExisting;

// not mapped automatically, as we include multiple entities like the plan_version and the price components
#[derive(Debug, o2o)]
#[owned_into(diesel_models::plans::PlanNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct PlanNew {
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
    pub external_id: String,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
}

pub struct FullPlanNew {
    pub plan: PlanNew,
    pub version: PlanVersionNewInternal,
    pub price_components: Vec<PriceComponentNewInternal>,
}

#[derive(Debug, o2o)]
#[owned_into_existing(diesel_models::plan_versions::PlanVersionNew)]
#[ghosts(id: {uuid::Uuid::now_v7()}, version: {0})]
pub struct PlanVersionNewInternal {
    pub is_draft_version: bool,
    pub trial_duration_days: Option<i32>,
    pub trial_fallback_plan_id: Option<Uuid>,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    #[into(~.into_iter().map(|v| Some(v)).collect())]
    pub billing_periods: Vec<BillingPeriodEnum>,
}

#[derive(Debug, o2o)]
#[owned_into(diesel_models::plan_versions::PlanVersionNew)]
pub struct PlanVersionNew {
    pub plan_id: Uuid,
    pub version: i32, // TODO check if it doesn't get overridden by the ghost
    #[parent]
    pub internal: PlanVersionNewInternal,
}

#[derive(Debug, o2o)]
#[owned_into_existing(diesel_models::price_components::PriceComponentNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct PriceComponentNewInternal {
    pub name: String,
    pub fee: serde_json::Value,
    pub product_item_id: Option<Uuid>,
    pub billable_metric_id: Option<Uuid>,
}

#[derive(Debug, o2o)]
#[owned_into(diesel_models::price_components::PriceComponentNew)]
pub struct PriceComponentNew {
    pub plan_version_id: Uuid,
    #[parent]
    pub internal: PriceComponentNewInternal,
}

#[derive(Debug, o2o)]
#[from_owned(diesel_models::plans::Plan)]
pub struct Plan {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub created_at: NaiveDateTime,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
    pub external_id: String,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
}

#[derive(Debug, o2o)]
#[from_owned(diesel_models::plan_versions::PlanVersion)]
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
    #[from(~.into_iter().filter_map(|v| v).collect())]
    pub billing_periods: Vec<BillingPeriodEnum>,
}

#[derive(Debug, o2o)]
#[from_owned(diesel_models::price_components::PriceComponent)]
pub struct PriceComponent {
    pub id: Uuid,
    pub name: String,
    pub fee: serde_json::Value,
    pub plan_version_id: Uuid,
    pub product_item_id: Option<Uuid>,
    pub billable_metric_id: Option<Uuid>,
}

pub struct FullPlan {
    pub plan: Plan,
    pub version: PlanVersion,
    pub price_components: Vec<PriceComponent>,
}
