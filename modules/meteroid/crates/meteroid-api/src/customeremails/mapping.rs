use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common_domain::ids::{BaseId, EntityActivityId, InvoicingEntityId};
use meteroid_grpc::meteroid::api::activity::v1::SentEmailAttachment as ProtoSentEmailAttachment;
use meteroid_grpc::meteroid::api::customeremails::v1 as server;
use meteroid_store::domain::InvoicingEntity;
use meteroid_store::domain::customer_emails::{
    EmailAudienceMode, EmailKind, EmailKindAudience, EmailKindAudiencePreview, EmailSettings,
    EmailSettingsPatch,
};
use meteroid_store::domain::entity_activity::EntityType;
use meteroid_store::domain::payment_reminders::EmailCopy;
use meteroid_store::repositories::customer_emails::{
    SentEmailCursor, SentEmailSummary, SkippedEmailSummary,
};
use meteroid_store::services::customer_locale::customer_enabled_locales;
use std::str::FromStr;
use strum::IntoEnumIterator;
use tonic::Status;
use uuid::Uuid;

use meteroid_app::conversions::ProtoConv;

pub fn kind_to_proto(kind: EmailKind) -> server::EmailKind {
    match kind {
        EmailKind::InvoiceReady => server::EmailKind::InvoiceReady,
        EmailKind::InvoicePaid => server::EmailKind::InvoicePaid,
        EmailKind::CreditNoteIssued => server::EmailKind::CreditNoteIssued,
        EmailKind::PaymentFailed => server::EmailKind::PaymentFailed,
        EmailKind::AuthenticationRequired => server::EmailKind::AuthenticationRequired,
        EmailKind::PaymentReminder => server::EmailKind::PaymentReminder,
        EmailKind::QuoteReady => server::EmailKind::QuoteReady,
        EmailKind::WalletBalanceLow => server::EmailKind::WalletBalanceLow,
        EmailKind::AutoTopUpFailed => server::EmailKind::AutoTopUpFailed,
        EmailKind::BudgetThreshold => server::EmailKind::BudgetThreshold,
        EmailKind::BudgetExceeded => server::EmailKind::BudgetExceeded,
    }
}

pub fn kind_from_proto(raw: i32) -> Result<EmailKind, Status> {
    let proto = server::EmailKind::try_from(raw)
        .map_err(|_| Status::invalid_argument(format!("unknown email kind {raw}")))?;
    match proto {
        server::EmailKind::Unspecified => Err(Status::invalid_argument("kind is required")),
        server::EmailKind::InvoiceReady => Ok(EmailKind::InvoiceReady),
        server::EmailKind::InvoicePaid => Ok(EmailKind::InvoicePaid),
        server::EmailKind::CreditNoteIssued => Ok(EmailKind::CreditNoteIssued),
        server::EmailKind::PaymentFailed => Ok(EmailKind::PaymentFailed),
        server::EmailKind::AuthenticationRequired => Ok(EmailKind::AuthenticationRequired),
        server::EmailKind::PaymentReminder => Ok(EmailKind::PaymentReminder),
        server::EmailKind::QuoteReady => Ok(EmailKind::QuoteReady),
        server::EmailKind::WalletBalanceLow => Ok(EmailKind::WalletBalanceLow),
        server::EmailKind::AutoTopUpFailed => Ok(EmailKind::AutoTopUpFailed),
        server::EmailKind::BudgetThreshold => Ok(EmailKind::BudgetThreshold),
        server::EmailKind::BudgetExceeded => Ok(EmailKind::BudgetExceeded),
    }
}

pub fn kind_to_i32(kind: Option<EmailKind>) -> i32 {
    kind.map_or(server::EmailKind::Unspecified, kind_to_proto) as i32
}

pub fn mode_from_proto(raw: i32) -> Result<EmailAudienceMode, Status> {
    match server::EmailAudienceMode::try_from(raw) {
        Ok(server::EmailAudienceMode::Exclude) => Ok(EmailAudienceMode::Exclude),
        Ok(server::EmailAudienceMode::Only) => Ok(EmailAudienceMode::Only),
        Ok(server::EmailAudienceMode::Unspecified) => {
            Err(Status::invalid_argument("mode is required"))
        }
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown audience mode {raw}"
        ))),
    }
}

fn mode_to_proto(mode: EmailAudienceMode) -> server::EmailAudienceMode {
    match mode {
        EmailAudienceMode::Exclude => server::EmailAudienceMode::Exclude,
        EmailAudienceMode::Only => server::EmailAudienceMode::Only,
    }
}

fn kind_audience_to_proto(rule: &EmailKindAudience) -> Result<server::EmailKindAudience, Status> {
    let audience = rule
        .audience
        .to_api_json()
        .map_err(|e| Status::internal(format!("failed to serialize audience: {e}")))?;
    Ok(server::EmailKindAudience {
        mode: mode_to_proto(rule.mode) as i32,
        audience,
    })
}

pub fn settings_to_proto(
    settings: &EmailSettings,
    audiences: &[EmailKindAudience],
    from_address: String,
    entity: &InvoicingEntity,
) -> Result<server::EmailSettings, Status> {
    let locales = customer_enabled_locales(entity);
    let kinds = EmailKind::iter()
        .map(|kind| {
            let audience = audiences
                .iter()
                .find(|a| a.kind == kind)
                .map(kind_audience_to_proto)
                .transpose()?;
            Ok(server::EmailKindInfo {
                kind: kind_to_proto(kind) as i32,
                enabled: settings.is_enabled(kind),
                editable_copy: kind.editable_copy(),
                attaches_document: kind.attaches_document(),
                audience,
                customer_level_audience: kind.customer_level_audience(),
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    Ok(server::EmailSettings {
        invoicing_entity_id: settings.invoicing_entity_id.as_proto(),
        sender_name: settings.sender_name.clone(),
        from_address,
        reply_to: settings.reply_to.clone(),
        bcc: settings.bcc.clone(),
        kinds,
        timezone: settings.timezone.clone(),
        reminder_send_hour: u32::from(settings.reminder_send_hour),
        // Read-only mirror of the entity's customer language: default first, then the extras.
        effective_locale: locales[0].to_string(),
        extra_locales: locales[1..].iter().map(|l| l.to_string()).collect(),
    })
}

pub fn audience_preview_to_proto(p: EmailKindAudiencePreview) -> server::EmailKindAudiencePreview {
    let n = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    server::EmailKindAudiencePreview {
        scanned_invoices: n(p.scanned_invoices),
        scanned_customers: n(p.scanned_customers),
        matched_invoices: n(p.matched_invoices),
        matched_customers: n(p.matched_customers),
        receiving_invoices: n(p.receiving_invoices),
        receiving_customers: n(p.receiving_customers),
    }
}

/// `""` clears a scalar; an absent field is left untouched.
pub fn patch_from_proto(data: server::EmailSettingsData) -> Result<EmailSettingsPatch, Status> {
    let clearable = |v: Option<String>| v.map(|s| Some(s).filter(|s| !s.trim().is_empty()));
    let reminder_send_hour = data
        .reminder_send_hour
        .map(|h| {
            u8::try_from(h)
                .map_err(|_| Status::invalid_argument("reminder_send_hour must be 0..=23"))
        })
        .transpose()?;
    Ok(EmailSettingsPatch {
        sender_name: clearable(data.sender_name),
        reply_to: clearable(data.reply_to),
        bcc: data.bcc.map(|l| l.values),
        timezone: clearable(data.timezone),
        reminder_send_hour,
    })
}

pub fn copy_from_proto(copy: Option<server::EmailCopy>) -> Option<EmailCopy> {
    copy.map(|c| EmailCopy {
        subject: c.subject,
        body: c.body,
    })
}

pub fn entity_id_from_proto(raw: &str) -> Result<InvoicingEntityId, Status> {
    InvoicingEntityId::from_proto(raw)
}

pub fn summary_to_proto(
    s: SentEmailSummary,
    entity_label: Option<String>,
    customer_name: Option<String>,
) -> server::SentEmailSummary {
    let entity_id = EntityType::from_str(&s.entity_type)
        .map(|t| t.id_as_proto(s.entity_id))
        .unwrap_or_else(|_| s.entity_id.to_string());
    server::SentEmailSummary {
        id: s.id.as_proto(),
        sent_at: s.sent_at.as_proto(),
        kind: kind_to_i32(s.kind),
        subject: s.subject,
        recipients: s.recipients,
        locale: s.locale,
        entity_type: s.entity_type,
        entity_id,
        entity_label,
        customer_id: s.customer_id.map(|c| c.as_proto()),
        customer_name,
        resent_from: s.resent_from.map(|r| r.as_proto()),
    }
}

pub fn skipped_summary_to_proto(
    s: SkippedEmailSummary,
    entity_label: Option<String>,
    customer_name: Option<String>,
) -> server::SkippedEmailSummary {
    let entity_id = EntityType::from_str(&s.entity_type)
        .map(|t| t.id_as_proto(s.entity_id))
        .unwrap_or_else(|_| s.entity_id.to_string());
    server::SkippedEmailSummary {
        id: s.id.as_proto(),
        skipped_at: s.skipped_at.as_proto(),
        kind: kind_to_i32(s.kind),
        reason: s.reason,
        entity_type: s.entity_type,
        entity_id,
        entity_label,
        customer_id: s.customer_id.map(|c| c.as_proto()),
        customer_name,
    }
}

/// Stored attachment JSON shape; `id` is absent on rows written before object ids were tracked.
#[derive(serde::Deserialize)]
struct StoredAttachment {
    filename: String,
    #[serde(default)]
    id: Option<String>,
}

pub fn parse_attachments(value: Option<&serde_json::Value>) -> Vec<ProtoSentEmailAttachment> {
    let Some(value) = value else {
        return vec![];
    };
    serde_json::from_value::<Vec<StoredAttachment>>(value.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|a| ProtoSentEmailAttachment {
            id: a.id.unwrap_or_default(),
            filename: a.filename,
        })
        .collect()
}

// base64url(sent_at_micros_i64_be || uuid_bytes), exactly like the activity cursor.
pub fn encode_cursor(c: SentEmailCursor) -> String {
    let micros = c.sent_at.and_utc().timestamp_micros();
    let mut buf = [0u8; 8 + 16];
    buf[..8].copy_from_slice(&micros.to_be_bytes());
    buf[8..].copy_from_slice(c.id.as_uuid().as_bytes());
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn decode_cursor(s: &str) -> Result<SentEmailCursor, Status> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| Status::invalid_argument("Invalid cursor"))?;
    if bytes.len() != 24 {
        return Err(Status::invalid_argument("Invalid cursor"));
    }
    let micros = i64::from_be_bytes(
        bytes[..8]
            .try_into()
            .map_err(|_| Status::invalid_argument("Invalid cursor"))?,
    );
    let sent_at = chrono::DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| Status::invalid_argument("Invalid cursor"))?
        .naive_utc();
    let id =
        Uuid::from_slice(&bytes[8..]).map_err(|_| Status::invalid_argument("Invalid cursor"))?;
    Ok(SentEmailCursor {
        sent_at,
        id: EntityActivityId::from(id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips() {
        let cursor = SentEmailCursor {
            sent_at: chrono::DateTime::from_timestamp_micros(1_700_000_000_123_456)
                .unwrap()
                .naive_utc(),
            id: EntityActivityId::new(),
        };
        let decoded = decode_cursor(&encode_cursor(cursor)).unwrap();
        assert_eq!(decoded.sent_at, cursor.sent_at);
        assert_eq!(decoded.id, cursor.id);
        assert!(decode_cursor("nope").is_err());
    }
}
