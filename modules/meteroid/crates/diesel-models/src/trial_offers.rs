//! Trial offers and the trials given (R-153, R-159); plan phase templates (R-154).
use chrono::NaiveDateTime;
use common_domain::ids::{
    CustomerId, PlanId, PlanVersionId, ProductFamilyId, SubscriptionId, TenantId, TrialOfferId,
};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::trial_offer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TrialOfferRow {
    pub id: TrialOfferId,
    pub tenant_id: TenantId,
    pub name: String,
    pub code: Option<String>,
    pub duration_days: i32,
    pub free: bool,
    pub trial_plan_id: Option<PlanId>,
    pub end_action: String,
    pub fallback_plan_version_id: Option<PlanVersionId>,
    pub eligibility_scope: String,
    pub archived_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// A trial given at a subscription's creation, with the plan and family it started on (R-159).
#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::trial_redemption)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TrialRedemptionRow {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub trial_offer_id: Option<TrialOfferId>,
    pub plan_id: PlanId,
    pub product_family_id: ProductFamilyId,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::plan_version_phase)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionPhaseRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub plan_version_id: PlanVersionId,
    pub seq: i32,
    pub offset_months: i32,
    pub label: Option<String>,
    pub terms: serde_json::Value,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::quote_phase)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuotePhaseRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub quote_id: common_domain::ids::QuoteId,
    pub seq: i32,
    pub offset_months: i32,
    pub label: Option<String>,
    pub terms: serde_json::Value,
    pub created_at: NaiveDateTime,
}
