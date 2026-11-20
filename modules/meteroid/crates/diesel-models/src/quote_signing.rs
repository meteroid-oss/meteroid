use chrono::NaiveDateTime;
use common_domain::ids::{
    QuoteEvidenceEventId, QuoteId, QuoteSignatureOtpId, StoredDocumentId, TenantId,
};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote_evidence_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteEvidenceEventRow {
    pub id: QuoteEvidenceEventId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub seq: i32,
    pub kind: String,
    pub actor_email: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub revision: i32,
    pub document_hash: Option<String>,
    pub details: serde_json::Value,
    pub occurred_at: NaiveDateTime,
    pub prev_hash: Option<String>,
    pub hash: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::quote_evidence_event)]
pub struct QuoteEvidenceEventRowNew {
    pub id: QuoteEvidenceEventId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub seq: i32,
    pub kind: String,
    pub actor_email: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub revision: i32,
    pub document_hash: Option<String>,
    pub details: serde_json::Value,
    pub occurred_at: NaiveDateTime,
    pub prev_hash: Option<String>,
    pub hash: String,
}

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote_signature_otp)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteSignatureOtpRow {
    pub id: QuoteSignatureOtpId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub email: String,
    pub code_hash: String,
    pub attempts: i32,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub verified_at: Option<NaiveDateTime>,
    pub consumed_at: Option<NaiveDateTime>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::quote_signature_otp)]
pub struct QuoteSignatureOtpRowNew {
    pub id: QuoteSignatureOtpId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub email: String,
    pub code_hash: String,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::quote_signature_certificate)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteSignatureCertificateRow {
    pub quote_id: QuoteId,
    pub tenant_id: TenantId,
    pub revision: i32,
    pub document_hash: String,
    pub evidence_hash: String,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub pdf_hash: Option<String>,
    pub seal_key_id: Option<String>,
    pub seal_signature: Option<String>,
    pub timestamp_authority: Option<String>,
    pub timestamp_token: Option<String>,
    pub created_at: NaiveDateTime,
    pub document_included: bool,
}
