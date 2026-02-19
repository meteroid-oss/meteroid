use chrono::NaiveDateTime;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plan_versions::PlanVersionRowNew;
use diesel_models::plan_versions::PlanVersionRowOverview;
use diesel_models::plan_versions::PlanVersionRowPatch;
use diesel_models::plans::PlanFilters as PlanFiltersDb;
use diesel_models::plans::PlanRow;
use diesel_models::plans::PlanRowForSubscription;
use diesel_models::plans::PlanRowNew;
use diesel_models::plans::PlanRowOverview;
use diesel_models::plans::PlanRowPatch;
use diesel_models::plans::PlanVersionRowInfo;
use diesel_models::plans::PlanWithVersionRow;

use super::enums::{PlanStatusEnum, PlanTypeEnum};
use common_domain::ids::{BaseId, PlanId, PlanVersionId, ProductFamilyId, ProductId, TenantId};
use o2o::o2o;
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::ProductFamilyOverview;
use crate::domain::price_components::{PriceComponent, PriceComponentNewInternal};
use crate::domain::products::Product;

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
    pub product_family_id: ProductFamilyId,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
}

impl PlanNew {
    pub fn into_raw(self, product_family_id: ProductFamilyId) -> PlanRowNew {
        PlanRowNew {
            id: PlanId::new(),
            name: self.name,
            description: self.description,
            created_by: self.created_by,
            tenant_id: self.tenant_id,
            product_family_id,
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
    // which plan is resolved during trial (if different from the current plan)
    pub trialing_plan_id: Option<PlanId>,
    // if true, the trial is free (no charges). If false, normal plan price applies.
    pub trial_is_free: bool,
}

#[derive(Debug, Clone)]
pub struct PlanVersionNew {
    pub plan_id: PlanId,
    pub created_by: Uuid,
    pub version: i32,
    pub tenant_id: TenantId,
    pub internal: PlanVersionNewInternal,
}

impl PlanVersionNew {
    pub fn into_raw(self, tenant_currency: String) -> PlanVersionRowNew {
        PlanVersionRowNew {
            id: PlanVersionId::new(),
            plan_id: self.plan_id,
            created_by: self.created_by,
            version: self.version,
            tenant_id: self.tenant_id,
            is_draft_version: self.internal.is_draft_version,
            trial_duration_days: self.internal.trial.as_ref().map(|v| v.duration_days as i32),
            trialing_plan_id: self
                .internal
                .trial
                .as_ref()
                .and_then(|v| v.trialing_plan_id),
            trial_is_free: self
                .internal
                .trial
                .as_ref()
                .is_some_and(|v| v.trial_is_free),
            period_start_day: self.internal.period_start_day,
            net_terms: self.internal.net_terms,
            currency: self.internal.currency.unwrap_or(tenant_currency),
            billing_cycles: self.internal.billing_cycles,
            uses_product_pricing: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, o2o)]
#[from_owned(PlanRow)]
pub struct Plan {
    pub id: PlanId,
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    #[from(~.into())]
    pub plan_type: PlanTypeEnum,
    #[from(~.into())]
    pub status: PlanStatusEnum,
    pub active_version_id: Option<PlanVersionId>,
    pub draft_version_id: Option<PlanVersionId>,
}

#[derive(Debug, Clone, PartialEq, o2o)]
#[from_owned(PlanVersionRow)]
pub struct PlanVersion {
    pub id: PlanVersionId,
    pub is_draft_version: bool,
    pub plan_id: PlanId,
    pub version: i32,
    pub tenant_id: TenantId,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub trialing_plan_id: Option<PlanId>,
    pub trial_is_free: bool,
    pub trial_duration_days: Option<i32>,
    pub uses_product_pricing: bool,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(PlanRowOverview)]
pub struct PlanOverview {
    pub id: PlanId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    #[from(~.into())]
    pub plan_type: PlanTypeEnum,
    #[from(~.into())]
    pub status: PlanStatusEnum,
    pub product_family_name: String,
    pub product_family_id: ProductFamilyId,
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
    pub id: PlanVersionId,
    pub plan_id: PlanId,
    pub plan_name: String,
    pub version: i32,
    pub created_by: Uuid,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub product_family_id: ProductFamilyId,
    pub product_family_name: String,
    pub trialing_plan_id: Option<PlanId>,
    pub trial_is_free: bool,
    pub trial_duration_days: Option<i32>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(PlanVersionRowInfo)]
pub struct PlanVersionInfo {
    pub id: PlanVersionId,
    pub version: i32,
    pub trial_duration_days: Option<i32>,
    // add currency(-ies) ?
}

#[derive(Clone, Debug, o2o)]
#[from_owned(PlanRowForSubscription)]
pub struct PlanForSubscription {
    pub version_id: PlanVersionId,
    pub net_terms: i32,
    pub name: String,
    pub currency: String,
    #[from(~.into())]
    pub plan_type: PlanTypeEnum,
    pub trial_duration_days: Option<i32>,
    pub trial_is_free: bool,
    pub product_family_id: ProductFamilyId,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(PlanVersionRowPatch)]
pub struct PlanVersionPatch {
    pub id: PlanVersionId,
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
    pub id: PlanId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub active_version_id: Option<Option<PlanVersionId>>,
}

#[derive(Debug, Clone, PartialEq, o2o)]
#[from_owned(PlanWithVersionRow)]
pub struct PlanWithVersion {
    #[from(~.into())]
    pub plan: Plan,
    #[from(~.map(| v | v.into()))]
    pub version: Option<PlanVersion>,
}

#[derive(Debug, Clone)]
pub struct FullPlan {
    pub plan: Plan,
    pub version: PlanVersion,
    pub price_components: Vec<PriceComponent>,
    pub product_family: ProductFamilyOverview,
    pub products: HashMap<ProductId, Product>,
}

pub struct TrialPatch {
    pub plan_version_id: PlanVersionId,
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
    pub filter_currency: Option<String>,
}
