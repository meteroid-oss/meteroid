//! A subscription's phases: dated stretches with their own terms, each written as the change of
//! its boundary. Billing reads the rows those changes wrote, never these.
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    AddOnId, ScheduledEventId, SubscriptionId, SubscriptionPhaseId, TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::subscription_phase)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionPhaseRow {
    pub id: SubscriptionPhaseId,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub seq: i32,
    pub start_date: NaiveDate,
    pub label: Option<String>,
    pub terms: serde_json::Value,
    pub change_id: ScheduledEventId,
    pub created_by: Option<Uuid>,
    pub created_at: NaiveDateTime,
    pub withdrawn_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::subscription_timeline_item)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionTimelineItemRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    /// `ADD_ON` or `DISCOUNT`.
    pub kind: String,
    pub add_on_id: Option<AddOnId>,
    /// An add-on's lineage, or the discounted component's.
    pub lineage_id: Uuid,
    pub start_date: NaiveDate,
    pub until_date: NaiveDate,
    pub terms: serde_json::Value,
    pub label: Option<String>,
    pub reason: Option<String>,
    pub start_change_id: Option<ScheduledEventId>,
    pub end_change_id: ScheduledEventId,
    pub created_by: Option<Uuid>,
    pub created_at: NaiveDateTime,
    pub withdrawn_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable, AsChangeset)]
#[diesel(table_name = crate::schema::subscription_term)]
#[diesel(primary_key(subscription_id))]
#[diesel(treat_none_as_null = true)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionTermRow {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub term_end: NaiveDate,
    pub auto_renew: bool,
    pub renewal_months: i32,
    pub uplift_percent: Option<rust_decimal::Decimal>,
    pub notice_days: i32,
    pub renewal_phase_id: Option<SubscriptionPhaseId>,
    pub created_by: Option<Uuid>,
    pub updated_at: NaiveDateTime,
}
