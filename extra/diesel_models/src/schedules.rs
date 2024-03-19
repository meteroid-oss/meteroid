


use uuid::Uuid;


use diesel::{Identifiable, Queryable};
use crate::enums::BillingPeriodEnum;



#[derive(Queryable, Debug)]
#[diesel(table_name = crate::schema::schedule)]
pub struct Schedule {
    pub id: Uuid,
    pub billing_period: BillingPeriodEnum,
    pub plan_version_id: Uuid,
    pub ramps: serde_json::Value,
}
