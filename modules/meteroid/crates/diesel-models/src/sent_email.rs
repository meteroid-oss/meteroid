use chrono::NaiveDateTime;
use common_domain::ids::{EntityActivityId, InvoicingEntityId, TenantId};
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
    /// `EmailKind` snake_case value; NULL on rows predating the column.
    pub kind: Option<String>,
    pub locale: Option<String>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    /// `EmailContext` JSON: what is needed to re-render.
    pub context: Option<serde_json::Value>,
    pub dedupe_key: Option<String>,
    pub resent_from: Option<EntityActivityId>,
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
    pub kind: Option<String>,
    pub locale: Option<String>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub context: Option<serde_json::Value>,
    pub dedupe_key: Option<String>,
    pub resent_from: Option<EntityActivityId>,
}

/// A `sent_email` row with the entity its `entity.email_sent` activity twin was recorded against.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::sent_email)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SentEmailWithEntityRow {
    #[diesel(embed)]
    pub email: SentEmailRow,
    #[diesel(select_expression = crate::schema::entity_activity::entity_type)]
    #[diesel(select_expression_type = crate::schema::entity_activity::entity_type)]
    pub entity_type: String,
    #[diesel(select_expression = crate::schema::entity_activity::entity_id)]
    #[diesel(select_expression_type = crate::schema::entity_activity::entity_id)]
    pub entity_id: uuid::Uuid,
    #[diesel(select_expression = crate::schema::entity_activity::agg_customer_id)]
    #[diesel(select_expression_type = crate::schema::entity_activity::agg_customer_id)]
    pub agg_customer_id: Option<uuid::Uuid>,
}
