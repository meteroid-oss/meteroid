use chrono::NaiveDateTime;
use common_domain::ids::{EntityActivityId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::sent_email)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SentEmailRow {
    pub id: EntityActivityId,
    pub tenant_id: TenantId,
    pub sent_at: NaiveDateTime,
    pub subject: String,
    pub from_addr: String,
    pub reply_to: Option<String>,
    /// Element-level Option<> is forced by diesel's TEXT[] mapping even
    /// though the column is `TEXT[] NOT NULL` and elements are never NULL.
    pub recipients: Vec<Option<String>>,
    pub body_html: String,
    pub attachments: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::sent_email)]
pub struct SentEmailRowNew {
    pub id: EntityActivityId,
    pub tenant_id: TenantId,
    pub subject: String,
    pub from_addr: String,
    pub reply_to: Option<String>,
    /// Element-level Option<> is forced by diesel's TEXT[] mapping even
    /// though the column is `TEXT[] NOT NULL` and elements are never NULL.
    pub recipients: Vec<Option<String>>,
    pub body_html: String,
    pub attachments: Option<serde_json::Value>,
}
