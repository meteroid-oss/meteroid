use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;
// TODO duplicate as well
use diesel_models::enums::{BillingPeriodEnum, PlanStatusEnum, PlanTypeEnum};

// not mapped automatically, as we include multiple entities like the plan_version and the price components
#[derive(Debug)]
pub struct PlanNew {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
    pub external_id: String,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub version_details: plan_new::PlanVersion,
}

pub mod plan_new {
    use diesel_models::enums::BillingPeriodEnum;
    use uuid::Uuid;

    #[derive(Debug)]
    pub struct PlanVersion {
        pub is_draft_version: bool,
        pub trial_duration_days: Option<i32>,
        pub trial_fallback_plan_id: Option<Uuid>,
        pub period_start_day: Option<i16>,
        pub net_terms: i32,
        pub currency: String,
        pub billing_cycles: Option<i32>,
        pub billing_periods: Vec<BillingPeriodEnum>,
        // price components etc
    }
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
