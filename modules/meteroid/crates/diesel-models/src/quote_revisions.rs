use chrono::NaiveDateTime;
use common_domain::ids::{QuoteId, QuoteRevisionId, StoredDocumentId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote_revision)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteRevisionRow {
    pub id: QuoteRevisionId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub revision: i32,
    pub snapshot: serde_json::Value,
    pub document_hash: String,
    pub created_by: Option<String>,
    pub created_at: NaiveDateTime,
    pub superseded_at: Option<NaiveDateTime>,
    pub pdf_document_id: Option<StoredDocumentId>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::quote_revision)]
pub struct QuoteRevisionRowNew {
    pub id: QuoteRevisionId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub revision: i32,
    pub snapshot: serde_json::Value,
    pub document_hash: String,
    pub created_by: Option<String>,
}
