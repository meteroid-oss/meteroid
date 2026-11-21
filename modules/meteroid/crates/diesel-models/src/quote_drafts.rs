use chrono::NaiveDateTime;
use common_domain::ids::{QuoteDraftId, QuoteId, TenantId, UserId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote_draft)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteDraftRow {
    pub id: QuoteDraftId,
    pub tenant_id: TenantId,
    pub quote_id: Option<QuoteId>,
    pub created_by: UserId,
    pub updated_by: Option<UserId>,
    pub label: Option<String>,
    pub payload: serde_json::Value,
    pub payload_version: i32,
    pub version: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::quote_draft)]
pub struct QuoteDraftRowNew {
    pub id: QuoteDraftId,
    pub tenant_id: TenantId,
    pub quote_id: Option<QuoteId>,
    pub created_by: UserId,
    pub updated_by: Option<UserId>,
    pub label: Option<String>,
    pub payload: serde_json::Value,
    pub payload_version: i32,
}
