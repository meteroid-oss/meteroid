//! A subscription's dated facts (Phase 7): its plan windows, trial, settled start and endings.
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{PlanVersionId, SubscriptionId, TenantId};
use diesel::{Insertable, Queryable, QueryableByName, Selectable};
use uuid::Uuid;

use crate::enums::{ActorTypeEnum, BillingPeriodEnum};

/// The plan version in force over `[from_date, to_date)`, and the grid its cadence bills on.
#[derive(Debug, Clone, Queryable, Selectable, QueryableByName)]
#[diesel(table_name = crate::schema::subscription_plan_window)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionPlanWindowRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub plan_version_id: PlanVersionId,
    pub from_date: NaiveDate,
    pub to_date: Option<NaiveDate>,
    pub cadence: BillingPeriodEnum,
    pub cadence_origin: NaiveDate,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::subscription_plan_window)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionPlanWindowRowNew {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub plan_version_id: PlanVersionId,
    pub from_date: NaiveDate,
    pub cadence: BillingPeriodEnum,
    pub cadence_origin: NaiveDate,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::subscription_trial)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionTrialRow {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub from_date: NaiveDate,
    pub until_date: NaiveDate,
    pub free: bool,
    pub created_at: NaiveDateTime,
}

/// Who settled a subscription's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartSettledBy {
    /// Its insert: a subscription starting on its start date.
    Start,
    /// A quote's conversion into one starting on its start date.
    Quote,
    Checkout,
    Payment,
    Manual,
}

impl StartSettledBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "START",
            Self::Quote => "QUOTE",
            Self::Checkout => "CHECKOUT",
            Self::Payment => "PAYMENT",
            Self::Manual => "MANUAL",
        }
    }
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::subscription_start_settled)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionStartSettledRow {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub at: NaiveDateTime,
    pub settled_by: String,
    pub created_at: NaiveDateTime,
}

pub const ENDING_CANCEL: &str = "CANCEL";
pub const ENDING_COMPLETE: &str = "COMPLETE";

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::subscription_ending)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionEndingRow {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub kind: String,
    pub at: NaiveDateTime,
    pub reason: Option<String>,
    pub actor_type: ActorTypeEnum,
    pub actor_uuid: Option<Uuid>,
    pub actor_alias: Option<String>,
    pub created_at: NaiveDateTime,
}
