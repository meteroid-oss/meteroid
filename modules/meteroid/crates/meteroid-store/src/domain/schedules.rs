use error_stack::Report;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::adjustments::discount::{Amount, StandardDiscount};
use crate::domain::enums::BillingPeriodEnum;
use crate::errors::StoreError;
use diesel_models::schedules::SchedulePatchRow;
use diesel_models::schedules::ScheduleRow;
use diesel_models::schedules::ScheduleRowNew;

#[derive(Clone, Debug)]
pub struct Schedule {
    pub id: Uuid,
    pub billing_period: BillingPeriodEnum,
    pub plan_version_id: Uuid,
    pub ramps: PlanRamps,
}

#[derive(Clone, Debug)]
pub struct ScheduleNew {
    pub billing_period: BillingPeriodEnum,
    pub plan_version_id: Uuid,
    pub ramps: PlanRamps,
}

#[derive(Clone, Debug)]
pub struct SchedulePatch {
    pub id: Uuid,
    pub ramps: Option<PlanRamps>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanRamps {
    pub ramps: Vec<PlanRamp>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanRamp {
    pub index: u32,
    pub duration_in_months: Option<u32>,
    pub ramp_adjustment: PlanRampAdjustment,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanRampAdjustment {
    pub minimum: Amount,
    pub discount: StandardDiscount,
}

impl TryFrom<serde_json::Value> for PlanRamps {
    type Error = Report<StoreError>;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        serde_json::from_value::<PlanRamps>(value).map_err(|e| {
            StoreError::SerdeError("Failed to deserialize plan ramps".to_string(), e).into()
        })
    }
}

impl TryInto<serde_json::Value> for PlanRamps {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<serde_json::Value, Self::Error> {
        serde_json::to_value(self).map_err(|e| {
            StoreError::SerdeError("Failed to serialize plan ramps".to_string(), e).into()
        })
    }
}

impl TryFrom<ScheduleRow> for Schedule {
    type Error = Report<StoreError>;

    fn try_from(value: ScheduleRow) -> Result<Self, Self::Error> {
        Ok(Schedule {
            id: value.id,
            billing_period: value.billing_period.into(),
            plan_version_id: value.plan_version_id,
            ramps: value.ramps.try_into()?,
        })
    }
}

impl TryInto<ScheduleRowNew> for ScheduleNew {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<ScheduleRowNew, Self::Error> {
        Ok(ScheduleRowNew {
            id: Uuid::now_v7(),
            billing_period: self.billing_period.into(),
            plan_version_id: self.plan_version_id,
            ramps: self.ramps.try_into()?,
        })
    }
}

impl TryInto<SchedulePatchRow> for SchedulePatch {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<SchedulePatchRow, Self::Error> {
        Ok(SchedulePatchRow {
            id: self.id,
            ramps: self.ramps.map(|r| r.try_into()).transpose()?,
        })
    }
}
