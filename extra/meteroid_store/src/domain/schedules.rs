use o2o::o2o;
use uuid::Uuid;

use crate::domain::enums::BillingPeriodEnum;
use diesel_models::schedules::Schedule as DieselSchedule;
use diesel_models::schedules::ScheduleNew as DieselScheduleNew;

#[derive(Clone, Debug, o2o)]
#[map_owned(DieselSchedule)]
pub struct Schedule {
    pub id: Uuid,
    #[map(~.into())]
    pub billing_period: BillingPeriodEnum,
    pub plan_version_id: Uuid,
    pub ramps: serde_json::Value, // TODO
}

#[derive(Clone, Debug, o2o)]
#[owned_into(DieselScheduleNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct ScheduleNew {
    #[into(~.into())]
    pub billing_period: BillingPeriodEnum,
    pub plan_version_id: Uuid,
    pub ramps: serde_json::Value,
}
