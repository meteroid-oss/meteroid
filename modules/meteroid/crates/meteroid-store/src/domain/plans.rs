use chrono::NaiveDateTime;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plan_versions::PlanVersionRowLatest;
use diesel_models::plan_versions::PlanVersionRowNew;
use diesel_models::plan_versions::PlanVersionRowPatch;
use diesel_models::plans::PlanRow;
use diesel_models::plans::PlanRowForList;
use diesel_models::plans::PlanRowNew;
use diesel_models::plans::PlanRowPatch;
use diesel_models::plans::PlanWithVersionRow;
use o2o::o2o;
use uuid::Uuid;
// TODO duplicate as well
use super::enums::{BillingPeriodEnum, PlanStatusEnum, PlanTypeEnum};

use crate::domain::price_components::{PriceComponent, PriceComponentNewInternal};

#[derive(Debug, Clone)]
pub struct PlanNew {
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub product_family_external_id: String,
    pub external_id: String,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
}

impl PlanNew {
    pub fn into_raw(self, product_family_id: Uuid) -> PlanRowNew {
        PlanRowNew {
            id: Uuid::now_v7(),
            name: self.name,
            description: self.description,
            created_by: self.created_by,
            tenant_id: self.tenant_id,
            product_family_id,
            external_id: self.external_id,
            plan_type: self.plan_type.into(),
            status: self.status.into(),
        }
    }
}

pub struct FullPlanNew {
    pub plan: PlanNew,
    pub version: PlanVersionNewInternal,
    pub price_components: Vec<PriceComponentNewInternal>,
}

#[derive(Debug, Clone)]
pub struct PlanVersionNewInternal {
    pub is_draft_version: bool,
    pub trial_duration_days: Option<i32>,
    pub trial_fallback_plan_id: Option<Uuid>,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: Option<String>,
    pub billing_cycles: Option<i32>,
    pub billing_periods: Vec<BillingPeriodEnum>,
}

#[derive(Debug, Clone)]
pub struct PlanVersionNew {
    pub plan_id: Uuid,
    pub created_by: Uuid,
    pub version: i32,
    pub tenant_id: Uuid,
    pub internal: PlanVersionNewInternal,
}

impl PlanVersionNew {
    pub fn into_raw(self, tenant_currency: String) -> PlanVersionRowNew {
        PlanVersionRowNew {
            id: Uuid::now_v7(),
            plan_id: self.plan_id,
            created_by: self.created_by,
            version: self.version,
            tenant_id: self.tenant_id,
            is_draft_version: self.internal.is_draft_version,
            trial_duration_days: self.internal.trial_duration_days,
            trial_fallback_plan_id: self.internal.trial_fallback_plan_id,
            period_start_day: self.internal.period_start_day,
            net_terms: self.internal.net_terms,
            currency: self.internal.currency.unwrap_or(tenant_currency),
            billing_cycles: self.internal.billing_cycles,
            billing_periods: self
                .internal
                .billing_periods
                .into_iter()
                .map(|v| v.into())
                .collect::<Vec<_>>(),
        }
    }
}

#[derive(Debug, o2o)]
#[from_owned(PlanRow)]
pub struct Plan {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub created_at: NaiveDateTime,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
    pub external_id: String,
    #[from(~.into())]
    pub plan_type: PlanTypeEnum,
    #[from(~.into())]
    pub status: PlanStatusEnum,
}

#[derive(Debug, o2o)]
#[from_owned(PlanVersionRow)]
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
    #[from(~.into_iter().map(| v | v.into()).collect::< Vec < _ >> ())]
    pub billing_periods: Vec<BillingPeriodEnum>,
}

pub struct FullPlan {
    pub plan: Plan,
    pub version: PlanVersion,
    pub price_components: Vec<PriceComponent>,
}

#[derive(Debug, o2o)]
#[from_owned(PlanRowForList)]
pub struct PlanForList {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
    pub external_id: String,
    #[from(~.into())]
    pub plan_type: PlanTypeEnum,
    #[from(~.into())]
    pub status: PlanStatusEnum,
    pub product_family_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, o2o)]
#[from_owned(PlanVersionRowLatest)]
pub struct PlanVersionLatest {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub plan_name: String,
    pub external_id: String,
    pub version: i32,
    pub created_by: Uuid,
    pub trial_duration_days: Option<i32>,
    pub trial_fallback_plan_id: Option<Uuid>,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub product_family_id: Uuid,
    pub product_family_name: String,
}

#[derive(Debug, o2o)]
#[owned_into(PlanVersionRowPatch)]
pub struct PlanVersionPatch {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub currency: Option<String>,
    pub net_terms: Option<i32>,
    #[into(~.map(| x | x.into_iter().map(| v | v.into()).collect::< Vec < _ >> ()))]
    pub billing_periods: Option<Vec<BillingPeriodEnum>>,
}

pub struct PlanAndVersionPatch {
    pub version: PlanVersionPatch,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
}

#[derive(Debug, o2o)]
#[owned_into(PlanRowPatch)]
pub struct PlanPatch {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
}

#[derive(Debug, o2o)]
#[from_owned(PlanWithVersionRow)]
pub struct PlanWithVersion {
    #[from(~.into())]
    pub plan: Plan,
    #[from(~.into())]
    pub version: PlanVersion,
}
