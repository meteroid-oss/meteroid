use chrono::NaiveDateTime;
use common_domain::ids::{QuoteEvidenceEventId, QuoteId, StoredDocumentId};
use diesel_models::quote_recipient_engagement::QuoteRecipientEngagementRow;
use diesel_models::quote_revisions::QuoteRevisionRow;
use diesel_models::quote_signing::{QuoteEvidenceEventRow, QuoteSignatureCertificateRow};
use diesel_models::quote_views::QuoteViewRow;
use serde::Serialize;

use crate::domain::quote_document::{QuoteDocument, hex_sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteEvidenceKind {
    Published,
    Revised,
    Sent,
    Viewed,
    CodeSent,
    CodeVerified,
    CodeRejected,
    Signed,
    Countersigned,
    Accepted,
    Declined,
    CertificateIssued,
    ChangesRequested,
}

impl QuoteEvidenceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Published => "published",
            Self::Revised => "revised",
            Self::Sent => "sent",
            Self::Viewed => "viewed",
            Self::CodeSent => "code_sent",
            Self::CodeVerified => "code_verified",
            Self::CodeRejected => "code_rejected",
            Self::Signed => "signed",
            Self::Countersigned => "countersigned",
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::CertificateIssued => "certificate_issued",
            Self::ChangesRequested => "changes_requested",
        }
    }
}

/// How a signature code reaches a signer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureCodeChannel {
    /// A test-mode workspace emails nothing: the portal shows the code.
    TestModeDisplay,
    Email,
    /// Emails are off or never reach the address.
    Unavailable,
}

impl SignatureCodeChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TestModeDisplay => "test_mode_display",
            Self::Email => "email",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Where a request came from, as recorded in the evidence.
#[derive(Debug, Clone, Default)]
pub struct RequestOrigin {
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QuoteEvidenceNew {
    pub kind: QuoteEvidenceKind,
    pub actor_email: Option<String>,
    pub origin: RequestOrigin,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct QuoteEvidenceEvent {
    pub id: QuoteEvidenceEventId,
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

impl From<QuoteEvidenceEventRow> for QuoteEvidenceEvent {
    fn from(r: QuoteEvidenceEventRow) -> Self {
        Self {
            id: r.id,
            quote_id: r.quote_id,
            seq: r.seq,
            kind: r.kind,
            actor_email: r.actor_email,
            ip_address: r.ip_address,
            user_agent: r.user_agent,
            revision: r.revision,
            document_hash: r.document_hash,
            details: r.details,
            occurred_at: r.occurred_at,
            prev_hash: r.prev_hash,
            hash: r.hash,
        }
    }
}

/// How often one person opened the quote: opens close together count as one view.
#[derive(Debug, Clone)]
pub struct QuoteRecipientEngagement {
    pub email: String,
    pub view_count: i32,
    pub first_viewed_at: NaiveDateTime,
    pub last_viewed_at: NaiveDateTime,
    pub last_viewed_revision: i32,
}

impl From<QuoteRecipientEngagementRow> for QuoteRecipientEngagement {
    fn from(r: QuoteRecipientEngagementRow) -> Self {
        Self {
            email: r.email,
            view_count: r.view_count,
            first_viewed_at: r.first_viewed_at,
            last_viewed_at: r.last_viewed_at,
            last_viewed_revision: r.last_viewed_revision,
        }
    }
}

/// Postgres stores `jsonb` objects with their keys reordered: hash them in one fixed order.
fn canonical(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            serde_json::Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), canonical(&map[k])))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonical).collect())
        }
        other => other.clone(),
    }
}

#[derive(Serialize)]
struct ChainedFields<'a> {
    seq: i32,
    kind: &'a str,
    actor_email: Option<&'a str>,
    ip_address: Option<&'a str>,
    user_agent: Option<&'a str>,
    revision: i32,
    document_hash: Option<&'a str>,
    details: serde_json::Value,
    occurred_at: String,
}

/// Each event's hash covers its fields and the previous hash, so editing or removing an event
/// breaks every hash after it.
#[allow(clippy::too_many_arguments)]
pub fn chain_hash(
    prev_hash: Option<&str>,
    seq: i32,
    kind: &str,
    actor_email: Option<&str>,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    revision: i32,
    document_hash: Option<&str>,
    details: &serde_json::Value,
    occurred_at: NaiveDateTime,
) -> String {
    let fields = ChainedFields {
        seq,
        kind,
        actor_email,
        ip_address,
        user_agent,
        revision,
        document_hash,
        details: canonical(details),
        occurred_at: occurred_at.and_utc().to_rfc3339(),
    };
    let mut bytes = prev_hash.unwrap_or_default().as_bytes().to_vec();
    bytes.push(b'|');
    bytes.extend(serde_json::to_vec(&fields).expect("evidence fields serialize"));
    hex_sha256(&bytes)
}

/// Whether the events, in order, form an unbroken chain.
pub fn evidence_chain_intact(events: &[QuoteEvidenceEvent]) -> bool {
    let mut prev: Option<&str> = None;
    events.iter().enumerate().all(|(i, e)| {
        let expected = chain_hash(
            prev,
            e.seq,
            &e.kind,
            e.actor_email.as_deref(),
            e.ip_address.as_deref(),
            e.user_agent.as_deref(),
            e.revision,
            e.document_hash.as_deref(),
            &e.details,
            e.occurred_at,
        );
        let ok = e.seq == i as i32 + 1 && e.prev_hash.as_deref() == prev && e.hash == expected;
        prev = Some(&e.hash);
        ok
    })
}

#[derive(Debug, Clone)]
pub struct QuoteRevision {
    pub quote_id: QuoteId,
    pub revision: i32,
    pub document: Option<QuoteDocument>,
    pub document_hash: String,
    pub created_by: Option<String>,
    pub created_at: NaiveDateTime,
    pub superseded_at: Option<NaiveDateTime>,
    pub pdf_document_id: Option<StoredDocumentId>,
}

impl From<QuoteRevisionRow> for QuoteRevision {
    fn from(r: QuoteRevisionRow) -> Self {
        Self {
            quote_id: r.quote_id,
            revision: r.revision,
            document: serde_json::from_value(r.snapshot).ok(),
            document_hash: r.document_hash,
            created_by: r.created_by,
            created_at: r.created_at,
            superseded_at: r.superseded_at,
            pdf_document_id: r.pdf_document_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuoteSignatureCertificate {
    pub quote_id: QuoteId,
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
    /// The PDF is the signed quote followed by these pages; standalone on rows issued before.
    pub document_included: bool,
}

impl From<QuoteSignatureCertificateRow> for QuoteSignatureCertificate {
    fn from(r: QuoteSignatureCertificateRow) -> Self {
        Self {
            quote_id: r.quote_id,
            revision: r.revision,
            document_hash: r.document_hash,
            evidence_hash: r.evidence_hash,
            pdf_document_id: r.pdf_document_id,
            pdf_hash: r.pdf_hash,
            seal_key_id: r.seal_key_id,
            seal_signature: r.seal_signature,
            timestamp_authority: r.timestamp_authority,
            timestamp_token: r.timestamp_token,
            created_at: r.created_at,
            document_included: r.document_included,
        }
    }
}

/// The message the seal signs: what was signed and the evidence of it, together.
pub fn seal_message(document_hash: &str, evidence_hash: &str, pdf_hash: Option<&str>) -> Vec<u8> {
    format!(
        "meteroid-quote-seal-v1|{document_hash}|{evidence_hash}|{}",
        pdf_hash.unwrap_or_default()
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_ignores_the_key_order_postgres_stores() {
        let at = chrono::DateTime::from_timestamp(1_700_000_000, 123_000)
            .unwrap()
            .naive_utc();
        let a = serde_json::json!({ "name": "Jane", "consent_text": "ok", "nested": { "b": 1, "a": 2 } });
        let b = serde_json::json!({ "nested": { "a": 2, "b": 1 }, "consent_text": "ok", "name": "Jane" });
        let hash = |details| chain_hash(None, 1, "signed", None, None, None, 1, None, details, at);
        assert_eq!(hash(&a), hash(&b));
    }
}

/// One sitting with the quote: opens within the view gap of the previous one extend it.
#[derive(Debug, Clone)]
pub struct QuoteViewSession {
    pub email: String,
    pub revision: i32,
    pub viewed_at: NaiveDateTime,
    pub last_seen_at: NaiveDateTime,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

impl From<QuoteViewRow> for QuoteViewSession {
    fn from(r: QuoteViewRow) -> Self {
        Self {
            email: r.email,
            revision: r.revision,
            viewed_at: r.viewed_at,
            last_seen_at: r.last_seen_at,
            ip_address: r.ip_address,
            user_agent: r.user_agent,
        }
    }
}
