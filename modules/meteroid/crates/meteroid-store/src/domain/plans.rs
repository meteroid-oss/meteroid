use chrono::NaiveDateTime;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plan_versions::PlanVersionRowNew;
use diesel_models::plan_versions::PlanVersionRowOverview;
use diesel_models::plan_versions::PlanVersionRowPatch;
use diesel_models::plans::PlanFilters as PlanFiltersDb;
use diesel_models::plans::PlanRow;
use diesel_models::plans::PlanRowNew;
use diesel_models::plans::PlanRowOverview;
use diesel_models::plans::PlanRowPatch;
use diesel_models::plans::PlanVersionRowInfo;
use diesel_models::plans::PlanWithVersionRow;

use common_domain::ids::TenantId;
use o2o::o2o;
use uuid::Uuid;
// TODO duplicate as well
use super::enums::{ActionAfterTrialEnum, PlanStatusEnum, PlanTypeEnum};

use crate::domain::price_components::{PriceComponent, PriceComponentNewInternal};

#[derive(Debug, Clone)]
pub enum PlanVersionFilter {
    Draft,
    Active,
    Version(i32),
}
impl From<PlanVersionFilter> for diesel_models::plan_versions::PlanVersionFilter {
    fn from(val: PlanVersionFilter) -> Self {
        match val {
            PlanVersionFilter::Draft => diesel_models::plan_versions::PlanVersionFilter::Draft,
            PlanVersionFilter::Active => diesel_models::plan_versions::PlanVersionFilter::Active,
            PlanVersionFilter::Version(v) => {
                diesel_models::plan_versions::PlanVersionFilter::Version(v)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanNew {
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub product_family_local_id: String,
    pub local_id: String,
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
            local_id: self.local_id,
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
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: Option<String>,
    pub billing_cycles: Option<i32>,
    pub trial: Option<PlanTrial>,
}

#[derive(Debug, Clone)]
pub struct PlanTrial {
    pub duration_days: u32,
    // which plan is resolved after trial ends (if different from the current plan)
    pub downgrade_plan_id: Option<Uuid>,
    // which plan is resolved during trial (if different from the current plan)
    pub trialing_plan_id: Option<Uuid>,
    pub action_after_trial: Option<ActionAfterTrialEnum>,
    pub require_pre_authorization: bool,
}

#[derive(Debug, Clone)]
pub struct PlanVersionNew {
    pub plan_id: Uuid,
    pub created_by: Uuid,
    pub version: i32,
    pub tenant_id: TenantId,
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
            trial_duration_days: self.internal.trial.as_ref().map(|v| v.duration_days as i32),
            action_after_trial: self
                .internal
                .trial
                .as_ref()
                .and_then(|v| v.action_after_trial.as_ref())
                .map(|v| v.clone().into()),
            downgrade_plan_id: self
                .internal
                .trial
                .as_ref()
                .and_then(|v| v.downgrade_plan_id),
            trialing_plan_id: self
                .internal
                .trial
                .as_ref()
                .and_then(|v| v.trialing_plan_id),
            trial_is_free: self
                .internal
                .trial
                .as_ref()
                .map(|v| v.require_pre_authorization)
                .unwrap_or(false),
            period_start_day: self.internal.period_start_day,
            net_terms: self.internal.net_terms,
            currency: self.internal.currency.unwrap_or(tenant_currency),
            billing_cycles: self.internal.billing_cycles,
        }
    }
}

#[derive(Debug, Clone, PartialEq, o2o)]
#[from_owned(PlanRow)]
pub struct Plan {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub product_family_id: Uuid,
    pub local_id: String,
    #[from(~.into())]
    pub plan_type: PlanTypeEnum,
    #[from(~.into())]
    pub status: PlanStatusEnum,
    pub active_version_id: Option<Uuid>,
    pub draft_version_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, o2o)]
#[from_owned(PlanVersionRow)]
pub struct PlanVersion {
    pub id: Uuid,
    pub is_draft_version: bool,
    pub plan_id: Uuid,
    pub version: i32,
    pub tenant_id: TenantId,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub trialing_plan_id: Option<Uuid>,
    #[from(~.map(| v | v.into()))]
    pub action_after_trial: Option<ActionAfterTrialEnum>,
    pub trial_is_free: bool,
    pub downgrade_plan_id: Option<Uuid>,
    pub trial_duration_days: Option<i32>,
}

pub struct FullPlan {
    pub plan: Plan,
    pub version: PlanVersion,
    pub price_components: Vec<PriceComponent>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(PlanRowOverview)]
pub struct PlanOverview {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub local_id: String,
    #[from(~.into())]
    pub plan_type: PlanTypeEnum,
    #[from(~.into())]
    pub status: PlanStatusEnum,
    pub product_family_name: String,
    pub product_family_local_id: String,
    #[from(~.map(| v | v.into()))]
    pub active_version: Option<PlanVersionInfo>,
    // pub draft_version: Option<Uuid>,
    #[from(draft_version, ~.is_some())]
    pub has_draft_version: bool,
    pub subscription_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, o2o)]
#[from_owned(PlanVersionRowOverview)]
pub struct PlanVersionOverview {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub plan_name: String,
    pub local_id: String,
    pub version: i32,
    pub created_by: Uuid,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub product_family_id: Uuid,
    pub product_family_name: String,
    pub trialing_plan_id: Option<Uuid>,
    #[from(~.map(| v | v.into()))]
    pub action_after_trial: Option<ActionAfterTrialEnum>,
    pub trial_is_free: bool,
    pub downgrade_plan_id: Option<Uuid>,
    pub trial_duration_days: Option<i32>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(PlanVersionRowInfo)]
pub struct PlanVersionInfo {
    pub version: i32,
    pub trial_duration_days: Option<i32>,
    // add currency(-ies) ?
}

#[derive(Clone, Debug, o2o)]
#[owned_into(PlanVersionRowPatch)]
pub struct PlanVersionPatch {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub currency: Option<String>,
    pub net_terms: Option<i32>,
}

pub struct PlanAndVersionPatch {
    pub version: PlanVersionPatch,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
}

#[derive(Debug, o2o)]
#[owned_into(PlanRowPatch)]
#[ghosts(draft_version_id: {None})]
pub struct PlanPatch {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub active_version_id: Option<Option<Uuid>>,
}

#[derive(Debug, Clone, PartialEq, o2o)]
#[from_owned(PlanWithVersionRow)]
pub struct PlanWithVersion {
    #[from(~.into())]
    pub plan: Plan,
    #[from(~.map(| v | v.into()))]
    pub version: Option<PlanVersion>,
}

pub struct TrialPatch {
    pub plan_version_id: Uuid,
    pub tenant_id: TenantId,
    pub trial: Option<PlanTrial>,
}

#[derive(Debug, o2o)]
#[owned_into(PlanFiltersDb)]
pub struct PlanFilters {
    pub search: Option<String>,
    #[into(~.into_iter().map(| v | v.into()).collect::< Vec < _ >> ())]
    pub filter_status: Vec<PlanStatusEnum>,
    #[into(~.into_iter().map(| v | v.into()).collect::< Vec < _ >> ())]
    pub filter_type: Vec<PlanTypeEnum>,
}
