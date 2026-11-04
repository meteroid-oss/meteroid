use chrono::NaiveDateTime;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

/// A version of a subscription's usage filter; no property means no filter from `effective_from`.
#[derive(Queryable, Debug, Clone, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::subscription_usage_filter)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionUsageFilterRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub property: Option<String>,
    pub value: Option<String>,
    pub effective_from: NaiveDateTime,
    pub effective_until: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::subscription_usage_filter)]
pub struct SubscriptionUsageFilterRowNew {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub property: Option<String>,
    pub value: Option<String>,
    pub effective_from: NaiveDateTime,
    pub effective_until: Option<NaiveDateTime>,
}
