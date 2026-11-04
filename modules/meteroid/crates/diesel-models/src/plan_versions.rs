use chrono::NaiveDateTime;

use common_domain::ids::{PlanId, PlanVersionId, ProductFamilyId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

use crate::enums::CreditExhaustedRuleEnum;

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionRow {
    pub id: PlanVersionId,
    pub is_draft_version: bool,
    pub plan_id: PlanId,
    pub version: i32,
    pub trial_duration_days: Option<i32>,
    pub tenant_id: TenantId,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    pub created_at: NaiveDateTime,
    pub trialing_plan_id: Option<PlanId>,
    pub trial_is_free: bool,
    pub uses_product_pricing: bool,
    pub tax_inclusive: bool,
    /// Unset on a draft whose rule was not chosen yet; publishing sets the default.
    pub credit_exhausted_rule: Option<CreditExhaustedRuleEnum>,
    /// With `InvoiceUpTo`: denomination code to minor units invoiced beyond credit per period.
    pub credit_exhausted_caps: serde_json::Value,
}

#[derive(Debug, Insertable, Default)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionRowNew {
    pub id: PlanVersionId,
    pub is_draft_version: bool,
    pub plan_id: PlanId,
    pub version: i32,
    pub trial_duration_days: Option<i32>,
    pub tenant_id: TenantId,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    pub trialing_plan_id: Option<PlanId>,
    pub trial_is_free: bool,
    pub uses_product_pricing: bool,
    pub tax_inclusive: bool,
    /// Unset on a draft whose rule was not chosen yet; publishing sets the default.
    pub credit_exhausted_rule: Option<CreditExhaustedRuleEnum>,
    /// With `InvoiceUpTo`: denomination code to minor units invoiced beyond credit per period.
    pub credit_exhausted_caps: serde_json::Value,
}

#[derive(Debug, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionRowOverview {
    pub id: PlanVersionId,
    pub plan_id: PlanId,
    #[diesel(select_expression = crate::schema::plan::name)]
    #[diesel(select_expression_type = crate::schema::plan::name)]
    pub plan_name: String,
    pub version: i32,
    pub trial_duration_days: Option<i32>,
    pub trialing_plan_id: Option<PlanId>,
    pub trial_is_free: bool,
    pub period_start_day: Option<i16>,
    pub net_terms: i32,
    pub currency: String,
    #[diesel(select_expression = crate::schema::product_family::id)]
    #[diesel(select_expression_type = crate::schema::product_family::id)]
    pub product_family_id: ProductFamilyId,
    #[diesel(select_expression = crate::schema::product_family::name)]
    #[diesel(select_expression_type = crate::schema::product_family::name)]
    pub product_family_name: String,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct PlanVersionRowPatch {
    pub id: PlanVersionId,
    pub tenant_id: TenantId,
    pub currency: Option<String>,
    pub net_terms: Option<i32>,
    pub tax_inclusive: Option<bool>,
    pub credit_exhausted_rule: Option<CreditExhaustedRuleEnum>,
    pub credit_exhausted_caps: Option<serde_json::Value>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::plan_version)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct PlanVersionTrialRowPatch {
    pub id: PlanVersionId,
    pub tenant_id: TenantId,
    pub trialing_plan_id: Option<Option<PlanId>>,
    pub trial_is_free: Option<bool>,
    pub trial_duration_days: Option<Option<i32>>,
}

#[derive(Debug, Clone)]
pub enum PlanVersionFilter {
    Draft,
    Active,
    Version(i32),
}
