use chrono::NaiveDateTime;
use common_domain::ids::{
    BaseId, ConnectorId, ConnectorSyncIssueId, CreditNoteId, CustomerId, InvoiceId,
};
use diesel_models::connector_sync_issues::ConnectorSyncIssueRow;

/// A document an integration syncs, as stored on a sync issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncResource {
    Customer(CustomerId),
    Invoice(InvoiceId),
    CreditNote(CreditNoteId),
}

impl SyncResource {
    pub fn parts(&self) -> (&'static str, uuid::Uuid) {
        match self {
            SyncResource::Customer(id) => ("customer", id.as_uuid()),
            SyncResource::Invoice(id) => ("invoice", id.as_uuid()),
            SyncResource::CreditNote(id) => ("credit_note", id.as_uuid()),
        }
    }

    fn from_parts(resource_type: &str, id: uuid::Uuid) -> Option<Self> {
        match resource_type {
            "customer" => Some(SyncResource::Customer(CustomerId::from(id))),
            "invoice" => Some(SyncResource::Invoice(InvoiceId::from(id))),
            "credit_note" => Some(SyncResource::CreditNote(CreditNoteId::from(id))),
            _ => None,
        }
    }
}

/// Why a document is not synced, kept until it syncs.
#[derive(Debug, Clone)]
pub struct ConnectorSyncIssue {
    pub id: ConnectorSyncIssueId,
    pub connector_id: ConnectorId,
    pub resource: SyncResource,
    pub message: String,
    pub occurrences: i32,
    pub first_seen_at: NaiveDateTime,
    pub last_seen_at: NaiveDateTime,
}

impl ConnectorSyncIssue {
    /// `None` for a resource type this version does not know.
    pub fn from_row(row: ConnectorSyncIssueRow) -> Option<Self> {
        Some(ConnectorSyncIssue {
            id: row.id,
            connector_id: row.connector_id,
            resource: SyncResource::from_parts(&row.resource_type, row.resource_id)?,
            message: row.message,
            occurrences: row.occurrences,
            first_seen_at: row.first_seen_at,
            last_seen_at: row.last_seen_at,
        })
    }
}
