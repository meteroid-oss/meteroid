use uuid::Uuid;

use crate::enums::BillingPeriodEnum;
use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug)]
#[diesel(table_name = crate::schema::schedule)]
pub struct Schedule {
    pub id: Uuid,
    pub billing_period: BillingPeriodEnum,
    pub plan_version_id: Uuid,
    pub ramps: serde_json::Value,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::schedule)]
pub struct ScheduleNew {
    pub id: Uuid,
    pub billing_period: BillingPeriodEnum,
    pub plan_version_id: Uuid,
    pub ramps: serde_json::Value,
}
