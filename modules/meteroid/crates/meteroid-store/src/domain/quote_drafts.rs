use chrono::NaiveDateTime;
use common_domain::ids::{QuoteDraftId, QuoteId, TenantId, UserId};
use diesel_models::quote_drafts::QuoteDraftRow;

/// Editor state saved as the user types, so a reload or an expired session loses nothing. The
/// payload is the web editor's own format, opaque to the server.
#[derive(Debug, Clone)]
pub struct QuoteDraft {
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

impl From<QuoteDraftRow> for QuoteDraft {
    fn from(row: QuoteDraftRow) -> Self {
        Self {
            id: row.id,
            tenant_id: row.tenant_id,
            quote_id: row.quote_id,
            created_by: row.created_by.into(),
            updated_by: row.updated_by.map(Into::into),
            label: row.label,
            payload: row.payload,
            payload_version: row.payload_version,
            version: row.version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuoteDraftSave {
    /// `None` starts a draft; a draft of an existing quote is found by `quote_id`.
    pub id: Option<QuoteDraftId>,
    pub quote_id: Option<QuoteId>,
    /// The version the editor loaded; `None` only when starting a draft.
    pub expected_version: Option<i32>,
    pub label: Option<String>,
    pub payload: serde_json::Value,
    pub payload_version: i32,
}

#[derive(Debug, Clone)]
pub enum QuoteDraftSaveOutcome {
    Saved(QuoteDraft),
    /// Someone saved a newer version: the editor gets it back to reload or overwrite.
    Conflict(QuoteDraft),
}
