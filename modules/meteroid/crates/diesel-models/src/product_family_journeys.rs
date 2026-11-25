use chrono::{DateTime, Utc};
use common_domain::ids::{PlanId, ProductFamilyId, TenantId};
use diesel::{Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::product_family_journey)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductFamilyJourneyRow {
    pub product_family_id: ProductFamilyId,
    pub tenant_id: TenantId,
    pub signup_plan_id: Option<PlanId>,
    /// The option's alias, kept across versions.
    pub signup_option: Option<String>,
    pub period_switch: bool,
    pub self_serve_downgrade: bool,
    pub updated_at: DateTime<Utc>,
}
