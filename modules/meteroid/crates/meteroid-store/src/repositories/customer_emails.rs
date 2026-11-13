//! Per-entity email settings, the claim-before-send state machine and the sent-email
//! reads behind `CustomerEmailsService`.

use crate::StoreResult;
use crate::domain::customer_emails::{
    AudienceSubject, EmailAudienceMode, EmailContext, EmailDelivery, EmailKind, EmailKindAudience,
    EmailKindAudiencePreview, EmailSettings, EmailSettingsPatch, MAX_BCC,
};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::payment_reminders::{Audience, SendWindow};
use crate::domain::pgmq::{PgmqMessageNew, PgmqQueue, SendEmailRequest};
use crate::errors::StoreError;
use crate::repositories::entity_activity::SentEmail;
use crate::repositories::pgmq::PgmqInterface;
use crate::services::payment_reminders::audience::{
    CompiledPredicate, check_customer_level, compile, evaluate_audience,
    evaluate_customer_audience, validate_audience,
};
use crate::store::{PgConn, Store, StoreInternal};
use chrono::NaiveDateTime;
use common_domain::ids::{
    BaseId, CustomerId, EntityActivityId, InvoiceId, InvoicingEntityId, TenantId,
};
use diesel_models::customers::CustomerRow;
use diesel_models::email_kind_audience::EmailKindAudienceRow;
use diesel_models::email_send_claim::EmailSendClaimRow;
use diesel_models::entity_activity::EntityActivityRow;
use diesel_models::enums::{EmailSendClaimStateEnum, InvoicePaymentStatus, InvoiceStatusEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::invoicing_entity_email_settings::{
    InvoicingEntityEmailSettingsRow, InvoicingEntityEmailSettingsRowNew,
};
use diesel_models::payment_reminders::InvoicePaymentReminderRow;
use diesel_models::query::payment_reminders::{
    count_distinct_customers, list_customer_ids_for_entity, list_recent_invoice_ids_for_entity,
};
pub use diesel_models::query::sent_email::{
    SentEmailCursor, SentEmailFilter, SkippedEmailCursor, SkippedEmailFilter,
};
use diesel_models::sent_email::{SentEmailRow, SentEmailWithEntityRow};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::HashSet;
use std::str::FromStr;
use uuid::Uuid;

pub const MAX_SENDER_NAME_CHARS: usize = 80;
pub const SENT_EMAILS_DEFAULT_LIMIT: u32 = 50;
pub const SENT_EMAILS_MAX_LIMIT: u32 = 200;
/// A `SENDING` claim older than this is no longer in flight: the sender died mid-send.
pub const CLAIM_IN_FLIGHT_WINDOW: chrono::Duration = chrono::Duration::minutes(15);
/// Invoices (or customers) a "Send to" preview evaluates at most, newest first.
const AUDIENCE_PREVIEW_SCAN_LIMIT: i64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    Won,
    /// `SENT` or `UNKNOWN`: terminal, never re-claimed.
    AlreadySent,
    /// `SENDING` inside the in-flight window: another sender owns the key.
    InFlight,
    /// Lost a race with a claim released since (`FAILED` or gone): nobody is sending.
    Released,
    /// `SENDING` past the in-flight window; the caller marks it `UNKNOWN`.
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimFinish {
    Sent,
    /// Released: the same key can be won again by a retried message.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentEmailSummary {
    pub id: EntityActivityId,
    pub sent_at: NaiveDateTime,
    pub kind: Option<EmailKind>,
    pub subject: String,
    pub recipients: Vec<String>,
    pub locale: Option<String>,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub customer_id: Option<CustomerId>,
    pub resent_from: Option<EntityActivityId>,
}

#[derive(Debug, Clone)]
pub struct SentEmailPage {
    pub items: Vec<SentEmailSummary>,
    pub next_cursor: Option<SentEmailCursor>,
}

/// A customer email that was not sent, from its `entity.email_skipped` /
/// `payment_reminder.skipped` activity entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEmailSummary {
    pub id: EntityActivityId,
    pub skipped_at: NaiveDateTime,
    pub kind: Option<EmailKind>,
    /// `SkipReason` snake_case value.
    pub reason: Option<String>,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub customer_id: Option<CustomerId>,
}

#[derive(Debug, Clone)]
pub struct SkippedEmailPage {
    pub items: Vec<SkippedEmailSummary>,
    pub next_cursor: Option<SkippedEmailCursor>,
}

/// One sent email with the platform columns and the entity its activity twin points at.
#[derive(Debug, Clone)]
pub struct SentEmailDetail {
    pub email: SentEmail,
    pub kind: Option<EmailKind>,
    pub locale: Option<String>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub context: Option<EmailContext>,
    pub dedupe_key: Option<String>,
    pub resent_from: Option<EntityActivityId>,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub customer_id: Option<CustomerId>,
}

impl SentEmailDetail {
    /// The context a resend re-renders from; legacy rows fall back to their invoice.
    pub fn resend_context(&self) -> Option<EmailContext> {
        if let Some(ctx) = &self.context {
            return Some(ctx.clone());
        }
        if self.entity_type == EntityType::Invoice.to_string() {
            return Some(EmailContext::Invoice {
                invoice_id: InvoiceId::from(self.entity_id),
            });
        }
        None
    }
}

fn parse_kind(kind: Option<&str>) -> Option<EmailKind> {
    kind.and_then(|k| EmailKind::from_str(k).ok())
}

impl TryFrom<SentEmailWithEntityRow> for SentEmailDetail {
    type Error = Report<StoreError>;

    fn try_from(row: SentEmailWithEntityRow) -> Result<Self, Self::Error> {
        let context = row
            .email
            .context
            .clone()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| {
                StoreError::SerdeError("Failed to deserialize sent_email context".into(), e)
            })?;
        Ok(Self {
            kind: parse_kind(row.email.kind.as_deref()),
            locale: row.email.locale.clone(),
            invoicing_entity_id: row.email.invoicing_entity_id,
            dedupe_key: row.email.dedupe_key.clone(),
            resent_from: row.email.resent_from,
            context,
            email: SentEmail::from(row.email),
            entity_type: row.entity_type,
            entity_id: row.entity_id,
            customer_id: row.agg_customer_id.map(CustomerId::from),
        })
    }
}

impl From<SentEmailWithEntityRow> for SentEmailSummary {
    fn from(row: SentEmailWithEntityRow) -> Self {
        let customer_id = row.agg_customer_id.or_else(|| {
            (row.entity_type == EntityType::Customer.to_string()).then_some(row.entity_id)
        });
        Self {
            id: row.email.id,
            sent_at: row.email.sent_at,
            kind: parse_kind(row.email.kind.as_deref()),
            subject: row.email.subject,
            recipients: row.email.recipients.into_iter().flatten().collect(),
            locale: row.email.locale,
            entity_type: row.entity_type,
            entity_id: row.entity_id,
            customer_id: customer_id.map(CustomerId::from),
            resent_from: row.email.resent_from,
        }
    }
}

impl From<EntityActivityRow> for SkippedEmailSummary {
    fn from(row: EntityActivityRow) -> Self {
        let text = |key: &str| {
            row.metadata
                .as_ref()
                .and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let reason = text("reason");
        let kind = parse_kind(text("kind").as_deref()).or_else(|| {
            (row.activity_type == ActivityType::PaymentReminderSkipped.to_string())
                .then_some(EmailKind::PaymentReminder)
        });
        Self {
            id: row.id,
            skipped_at: row.occurred_at,
            kind,
            reason,
            entity_type: row.entity_type,
            entity_id: row.entity_id,
            customer_id: row.agg_customer_id.map(CustomerId::from),
        }
    }
}

/// A stored rule that no longer parses is ignored (Everyone) rather than breaking the settings
/// page or retrying the send forever.
fn usable_rule(row: EmailKindAudienceRow) -> Option<(EmailKindAudience, CompiledPredicate)> {
    let (entity, kind) = (row.invoicing_entity_id, row.kind.clone());
    let parsed =
        EmailKindAudience::try_from(row).and_then(|rule| Ok((compile(&rule.audience)?, rule)));
    match parsed {
        Ok((compiled, rule)) => Some((rule, compiled)),
        Err(e) => {
            log::warn!("Ignoring unreadable {kind} audience rule of entity {entity}: {e:?}");
            None
        }
    }
}

fn invalid(msg: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(msg.into()))
}

/// The stored sender fields after applying `patch`, or `InvalidArgument`.
fn apply_patch(
    mut settings: EmailSettings,
    patch: EmailSettingsPatch,
) -> StoreResult<EmailSettings> {
    if let Some(sender_name) = patch.sender_name {
        let sender_name = sender_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(name) = &sender_name {
            if name.chars().count() > MAX_SENDER_NAME_CHARS {
                return Err(invalid(format!(
                    "sender_name must be at most {MAX_SENDER_NAME_CHARS} characters"
                )));
            }
            if name.contains(['<', '>', '\n', '\r']) {
                return Err(invalid(
                    "sender_name cannot contain '<', '>' or line breaks",
                ));
            }
        }
        settings.sender_name = sender_name;
    }
    if let Some(reply_to) = patch.reply_to {
        let reply_to = reply_to
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(addr) = &reply_to {
            meteroid_mailer::model::validate_email_address(addr)
                .map_err(|e| invalid(format!("reply_to is not a valid address: {e}")))?;
        }
        settings.reply_to = reply_to;
    }
    if let Some(bcc) = patch.bcc {
        let mut deduped: Vec<String> = Vec::new();
        for addr in bcc {
            let addr = addr.trim().to_lowercase();
            if addr.is_empty() {
                continue;
            }
            meteroid_mailer::model::validate_email_address(&addr)
                .map_err(|e| invalid(format!("bcc entry is not a valid address: {e}")))?;
            if !deduped.contains(&addr) {
                deduped.push(addr);
            }
        }
        if deduped.len() > MAX_BCC {
            return Err(invalid(format!("bcc can hold at most {MAX_BCC} addresses")));
        }
        settings.bcc = deduped;
    }
    if let Some(timezone) = patch.timezone {
        let timezone = timezone
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(tz) = &timezone {
            common_domain::identifiers::parse_timezone(tz)
                .map_err(|_| invalid(format!("timezone {tz:?} is not a valid IANA zone")))?;
        }
        settings.timezone = timezone;
    }
    if let Some(hour) = patch.reminder_send_hour {
        if hour > 23 {
            return Err(invalid("reminder_send_hour must be between 0 and 23"));
        }
        settings.reminder_send_hour = hour;
    }
    Ok(settings)
}

fn settings_row_new(tenant_id: TenantId, s: &EmailSettings) -> InvoicingEntityEmailSettingsRowNew {
    InvoicingEntityEmailSettingsRowNew {
        invoicing_entity_id: s.invoicing_entity_id,
        tenant_id,
        sender_name: s.sender_name.clone(),
        reply_to: s.reply_to.clone(),
        bcc: s.bcc.iter().cloned().map(Some).collect(),
        disabled_kinds: s
            .disabled_kinds
            .iter()
            .map(|k| Some(k.to_string()))
            .collect(),
        timezone: s.timezone.clone(),
        reminder_send_hour: i16::from(s.reminder_send_hour),
    }
}

fn changed_fields(before: &EmailSettings, after: &EmailSettings) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if before.sender_name != after.sender_name {
        fields.push("email_settings.sender_name");
    }
    if before.reply_to != after.reply_to {
        fields.push("email_settings.reply_to");
    }
    if before.bcc != after.bcc {
        fields.push("email_settings.bcc");
    }
    if before.timezone != after.timezone {
        fields.push("email_settings.timezone");
    }
    if before.reminder_send_hour != after.reminder_send_hour {
        fields.push("email_settings.reminder_send_hour");
    }
    fields
}

async fn distinct_customers(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoices: &[InvoiceId],
) -> StoreResult<usize> {
    count_distinct_customers(conn, tenant_id, invoices)
        .await
        .map(|c| usize::try_from(c).unwrap_or(0))
        .map_err(Into::<Report<StoreError>>::into)
}

/// Queues a customer email in the caller's transaction, for code that holds a connection only.
pub async fn enqueue_customer_email_on_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    kind: EmailKind,
    context: EmailContext,
    dedupe_key: String,
) -> StoreResult<()> {
    let msg = customer_email_message(tenant_id, kind, context, dedupe_key, None)?;
    diesel_models::query::pgmq::send_batch(
        conn,
        PgmqQueue::SendEmailRequest.as_str(),
        &[msg.into()],
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)
}

pub fn customer_email_message(
    tenant_id: TenantId,
    kind: EmailKind,
    context: EmailContext,
    dedupe_key: String,
    resent_from: Option<EntityActivityId>,
) -> StoreResult<PgmqMessageNew> {
    SendEmailRequest::CustomerEmail {
        tenant_id,
        kind,
        context,
        dedupe_key,
        resent_from,
        manual: false,
        attempts: Default::default(),
        recipient: None,
    }
    .try_into()
}

#[async_trait::async_trait]
pub trait CustomerEmailsInterface {
    /// Defaults when the entity has no stored row.
    async fn get_email_settings(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<EmailSettings>;

    async fn update_email_settings(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        patch: EmailSettingsPatch,
    ) -> StoreResult<EmailSettings>;

    async fn set_email_kind_enabled(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        enabled: bool,
    ) -> StoreResult<EmailSettings>;

    async fn list_sent_emails(
        &self,
        tenant_id: TenantId,
        filter: SentEmailFilter,
        cursor: Option<SentEmailCursor>,
        limit: u32,
    ) -> StoreResult<SentEmailPage>;

    async fn get_sent_email_detail(
        &self,
        tenant_id: TenantId,
        id: EntityActivityId,
    ) -> StoreResult<SentEmailDetail>;

    /// Customer emails that were not sent (their skip activity entries), newest first.
    async fn list_skipped_emails(
        &self,
        tenant_id: TenantId,
        filter: SkippedEmailFilter,
        cursor: Option<SkippedEmailCursor>,
        limit: u32,
    ) -> StoreResult<SkippedEmailPage>;

    /// Upserts the claim in its own committed transaction, before any transport call.
    async fn claim_email_send(
        &self,
        tenant_id: TenantId,
        dedupe_key: &str,
        now: NaiveDateTime,
    ) -> StoreResult<ClaimOutcome>;

    async fn finish_email_send(
        &self,
        tenant_id: TenantId,
        dedupe_key: &str,
        finish: ClaimFinish,
        sent_email_id: Option<EntityActivityId>,
    ) -> StoreResult<()>;

    /// The only writer of `UNKNOWN`; a no-op (`false`) unless the claim is still a stale `SENDING`.
    async fn mark_claim_unknown(&self, tenant_id: TenantId, dedupe_key: &str) -> StoreResult<bool>;

    /// The workspace's delivery mode and who `Test` mode reaches, read uncached: a change
    /// applies to the next message.
    async fn email_delivery(&self, tenant_id: TenantId) -> StoreResult<EmailDelivery>;

    async fn enqueue_customer_email(
        &self,
        tenant_id: TenantId,
        kind: EmailKind,
        context: EmailContext,
        dedupe_key: String,
    ) -> StoreResult<()>;

    /// Same, inside the caller's transaction (the message is visible only once it commits).
    async fn enqueue_customer_email_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        kind: EmailKind,
        context: EmailContext,
        dedupe_key: String,
    ) -> StoreResult<()>;

    async fn list_email_kind_audiences(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<Vec<EmailKindAudience>>;

    async fn set_email_kind_audience(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        mode: EmailAudienceMode,
        audience: Audience,
    ) -> StoreResult<EmailKindAudience>;

    /// Back to everyone; a no-op when the kind has no rule.
    async fn clear_email_kind_audience(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
    ) -> StoreResult<()>;

    /// Over the entity's recent finalized invoices, or its customers for customer-level kinds.
    async fn preview_email_kind_audience(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        mode: EmailAudienceMode,
        audience: &Audience,
    ) -> StoreResult<EmailKindAudiencePreview>;

    /// Whether the entity's "Send to" rule for `kind` keeps this email from going out, on live data.
    async fn email_audience_suppresses(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        subject: AudienceSubject,
    ) -> StoreResult<bool>;

    /// Re-renders a sent email from live data under a fresh key; idempotent per `request_id`.
    async fn resend_email(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        sent_email_id: EntityActivityId,
        request_id: &str,
    ) -> StoreResult<()>;

    /// Sends again an email whose delivery is unknown, from its activity entry, under a fresh
    /// key; idempotent per `request_id`, refused once the same email has been sent since.
    async fn resend_unknown_email(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        activity_id: EntityActivityId,
        request_id: &str,
    ) -> StoreResult<()>;
}

impl StoreInternal {
    async fn entity_in_tenant(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<InvoicingEntityRow> {
        InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
            conn,
            invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
    }

    async fn stored_settings(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<Option<InvoicingEntityEmailSettingsRow>> {
        InvoicingEntityEmailSettingsRow::find_by_entity_id(conn, tenant_id, invoicing_entity_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    pub(crate) async fn get_email_settings_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<EmailSettings> {
        match self
            .stored_settings(conn, tenant_id, invoicing_entity_id)
            .await?
        {
            Some(row) => Ok(row.into()),
            None => {
                self.entity_in_tenant(conn, tenant_id, invoicing_entity_id)
                    .await?;
                Ok(EmailSettings::defaults(invoicing_entity_id))
            }
        }
    }

    async fn validate_kind_audience(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        audience: &Audience,
    ) -> StoreResult<()> {
        if !kind.takes_audience_rule() {
            return Err(invalid(
                "Payment reminders target audiences through their reminder sequences",
            ));
        }
        self.entity_in_tenant(conn, tenant_id, invoicing_entity_id)
            .await?;
        if kind.customer_level_audience() {
            check_customer_level(audience)?;
        }
        validate_audience(conn, tenant_id, invoicing_entity_id, audience).await
    }

    /// Mode and conditions are separate changes so a conditions-only edit is visible in the log.
    async fn audit_kind_audience(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        actor: &Actor,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        before: Option<&EmailKindAudienceRow>,
        after: Option<&EmailKindAudienceRow>,
    ) -> StoreResult<()> {
        let field = format!("email_settings.audience.{kind}");
        let mode = |r: Option<&EmailKindAudienceRow>| r.map(|r| EmailAudienceMode::from(r.mode));
        let conditions = |r: Option<&EmailKindAudienceRow>| r.map(|r| r.audience.clone());
        let mut changes = Vec::new();
        if mode(before) != mode(after) {
            changes.push(serde_json::json!({
                "field": field,
                "from": mode(before),
                "to": mode(after),
            }));
        }
        if conditions(before) != conditions(after) {
            changes.push(serde_json::json!({
                "field": format!("{field}.conditions"),
                "from": conditions(before),
                "to": conditions(after),
            }));
        }
        if changes.is_empty() {
            return Ok(());
        }
        self.record_audit_tx(
            conn,
            tenant_id,
            actor,
            AuditInput::Activity(
                Activity::new(
                    ActivityType::EntityUpdated,
                    EntityType::InvoicingEntity,
                    invoicing_entity_id.as_uuid(),
                )
                .with_metadata(serde_json::json!({ "changes": changes })),
            ),
        )
        .await
    }

    /// Reminder fire times depend on the send window: in-flight invoices are re-stamped.
    pub(crate) async fn on_email_send_window_changed(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        window: SendWindow,
        now: chrono::NaiveDateTime,
    ) -> StoreResult<()> {
        self.restamp_reminders_for_entity(conn, tenant_id, invoicing_entity_id, window, now)
            .await
            .map(|_| ())
    }
}

#[async_trait::async_trait]
impl CustomerEmailsInterface for Store {
    async fn get_email_settings(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<EmailSettings> {
        let mut conn = self.get_conn().await?;
        self.internal
            .get_email_settings_tx(&mut conn, tenant_id, invoicing_entity_id)
            .await
    }

    async fn update_email_settings(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        patch: EmailSettingsPatch,
    ) -> StoreResult<EmailSettings> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let before = self
                    .internal
                    .get_email_settings_tx(conn, tenant_id, invoicing_entity_id)
                    .await?;
                let after = apply_patch(before.clone(), patch)?;
                let now = common_domain::clock::now().naive_utc();
                // `disabled_kinds` only seeds a missing row: an existing row keeps its own set.
                let row = settings_row_new(tenant_id, &after)
                    .upsert_settings(conn, now)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let stored: EmailSettings = row.into();

                let fields = changed_fields(&before, &stored);
                if !fields.is_empty() {
                    let changes: Vec<serde_json::Value> = fields
                        .iter()
                        .map(|f| serde_json::json!({ "field": f }))
                        .collect();
                    self.internal
                        .record_audit_tx(
                            conn,
                            tenant_id,
                            actor,
                            AuditInput::Activity(
                                Activity::new(
                                    ActivityType::EntityUpdated,
                                    EntityType::InvoicingEntity,
                                    invoicing_entity_id.as_uuid(),
                                )
                                .with_metadata(serde_json::json!({ "changes": changes })),
                            ),
                        )
                        .await?;
                }
                if before.timezone != stored.timezone
                    || before.reminder_send_hour != stored.reminder_send_hour
                {
                    self.internal
                        .on_email_send_window_changed(
                            conn,
                            tenant_id,
                            invoicing_entity_id,
                            stored.send_window(),
                            now,
                        )
                        .await?;
                }
                Ok(stored)
            }
            .scope_boxed()
        })
        .await
    }

    async fn set_email_kind_enabled(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        enabled: bool,
    ) -> StoreResult<EmailSettings> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let before = self
                    .internal
                    .get_email_settings_tx(conn, tenant_id, invoicing_entity_id)
                    .await?;
                let mut seed = before.clone();
                if enabled {
                    seed.disabled_kinds.remove(&kind);
                } else {
                    seed.disabled_kinds.insert(kind);
                }
                let now = common_domain::clock::now().naive_utc();
                let row = settings_row_new(tenant_id, &seed)
                    .set_kind_enabled(conn, &kind.to_string(), enabled, now)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let stored: EmailSettings = row.into();
                if before.is_enabled(kind) != stored.is_enabled(kind) {
                    self.internal
                        .record_audit_tx(
                            conn,
                            tenant_id,
                            actor,
                            AuditInput::Activity(
                                Activity::new(
                                    ActivityType::EntityUpdated,
                                    EntityType::InvoicingEntity,
                                    invoicing_entity_id.as_uuid(),
                                )
                                .with_metadata(serde_json::json!({
                                    "changes": [{
                                        "field": format!("email_settings.kinds.{kind}"),
                                        "from": !enabled,
                                        "to": enabled,
                                    }]
                                })),
                            ),
                        )
                        .await?;
                }
                Ok(stored)
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_sent_emails(
        &self,
        tenant_id: TenantId,
        filter: SentEmailFilter,
        cursor: Option<SentEmailCursor>,
        limit: u32,
    ) -> StoreResult<SentEmailPage> {
        let limit = limit.clamp(1, SENT_EMAILS_MAX_LIMIT);
        let mut conn = self.get_conn().await?;
        let mut rows =
            SentEmailRow::list(&mut conn, tenant_id, &filter, cursor, i64::from(limit) + 1)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let next_cursor = if rows.len() > limit as usize {
            rows.truncate(limit as usize);
            rows.last().map(|r| SentEmailCursor {
                sent_at: r.email.sent_at,
                id: r.email.id,
            })
        } else {
            None
        };
        Ok(SentEmailPage {
            items: rows.into_iter().map(Into::into).collect(),
            next_cursor,
        })
    }

    async fn get_sent_email_detail(
        &self,
        tenant_id: TenantId,
        id: EntityActivityId,
    ) -> StoreResult<SentEmailDetail> {
        let mut conn = self.get_conn().await?;
        SentEmailRow::find_with_entity_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "sent email {} not found",
                    id.as_base62()
                )))
            })?
            .try_into()
    }

    async fn list_skipped_emails(
        &self,
        tenant_id: TenantId,
        filter: SkippedEmailFilter,
        cursor: Option<SkippedEmailCursor>,
        limit: u32,
    ) -> StoreResult<SkippedEmailPage> {
        let limit = limit.clamp(1, SENT_EMAILS_MAX_LIMIT);
        let mut conn = self.get_conn().await?;
        let mut rows = EntityActivityRow::list_skipped_emails(
            &mut conn,
            tenant_id,
            &filter,
            cursor,
            i64::from(limit) + 1,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let next_cursor = if rows.len() > limit as usize {
            rows.truncate(limit as usize);
            rows.last().map(|r| SkippedEmailCursor {
                sent_at: r.occurred_at,
                id: r.id,
            })
        } else {
            None
        };
        Ok(SkippedEmailPage {
            items: rows.into_iter().map(Into::into).collect(),
            next_cursor,
        })
    }

    async fn claim_email_send(
        &self,
        tenant_id: TenantId,
        dedupe_key: &str,
        now: NaiveDateTime,
    ) -> StoreResult<ClaimOutcome> {
        let mut conn = self.get_conn().await?;
        if EmailSendClaimRow::claim(&mut conn, tenant_id, dedupe_key, now)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .is_some()
        {
            return Ok(ClaimOutcome::Won);
        }
        let existing = EmailSendClaimRow::find_by_dedupe_key(&mut conn, dedupe_key)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(match existing {
            // Lost a race with a claim that has since been released: the retried message
            // re-wins it.
            None => ClaimOutcome::Released,
            Some(row) if row.tenant_id != tenant_id => {
                return Err(Report::new(StoreError::InvalidArgument(
                    "dedupe key belongs to another tenant".to_string(),
                )));
            }
            Some(row) => match row.state {
                EmailSendClaimStateEnum::Sent | EmailSendClaimStateEnum::Unknown => {
                    ClaimOutcome::AlreadySent
                }
                EmailSendClaimStateEnum::Failed => ClaimOutcome::Released,
                EmailSendClaimStateEnum::Sending => {
                    if row.claimed_at > now - CLAIM_IN_FLIGHT_WINDOW {
                        ClaimOutcome::InFlight
                    } else {
                        ClaimOutcome::Stale
                    }
                }
            },
        })
    }

    async fn finish_email_send(
        &self,
        tenant_id: TenantId,
        dedupe_key: &str,
        finish: ClaimFinish,
        sent_email_id: Option<EntityActivityId>,
    ) -> StoreResult<()> {
        let state = match finish {
            ClaimFinish::Sent => EmailSendClaimStateEnum::Sent,
            ClaimFinish::Failed => EmailSendClaimStateEnum::Failed,
        };
        let mut conn = self.get_conn().await?;
        EmailSendClaimRow::finish(
            &mut conn,
            tenant_id,
            dedupe_key,
            state,
            sent_email_id,
            common_domain::clock::now().naive_utc(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    async fn mark_claim_unknown(&self, tenant_id: TenantId, dedupe_key: &str) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;
        let now = common_domain::clock::now().naive_utc();
        let updated = EmailSendClaimRow::mark_unknown(
            &mut conn,
            tenant_id,
            dedupe_key,
            now - CLAIM_IN_FLIGHT_WINDOW,
            now,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(updated > 0)
    }

    async fn email_delivery(&self, tenant_id: TenantId) -> StoreResult<EmailDelivery> {
        let mut conn = self.get_conn().await?;
        let tenant = diesel_models::tenants::TenantRow::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let mode: crate::domain::enums::EmailDeliveryMode = tenant.email_delivery_mode.into();
        let members = if mode == crate::domain::enums::EmailDeliveryMode::Test {
            diesel_models::users::UserRow::list_by_org_id(&mut conn, tenant.organization_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .filter(|u| u.archived_at.is_none())
                .map(|u| u.email)
                .collect()
        } else {
            vec![]
        };
        Ok(EmailDelivery {
            mode,
            members,
            allowlist: tenant.email_test_recipients,
        })
    }

    async fn enqueue_customer_email(
        &self,
        tenant_id: TenantId,
        kind: EmailKind,
        context: EmailContext,
        dedupe_key: String,
    ) -> StoreResult<()> {
        let msg = customer_email_message(tenant_id, kind, context, dedupe_key, None)?;
        self.pgmq_send_batch(PgmqQueue::SendEmailRequest, vec![msg])
            .await
    }

    async fn enqueue_customer_email_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        kind: EmailKind,
        context: EmailContext,
        dedupe_key: String,
    ) -> StoreResult<()> {
        let msg = customer_email_message(tenant_id, kind, context, dedupe_key, None)?;
        self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, vec![msg])
            .await
    }

    async fn list_email_kind_audiences(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<Vec<EmailKindAudience>> {
        let mut conn = self.get_conn().await?;
        let rows = EmailKindAudienceRow::list_by_entity(&mut conn, tenant_id, invoicing_entity_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| usable_rule(row).map(|(rule, _)| rule))
            .collect())
    }

    async fn set_email_kind_audience(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        mode: EmailAudienceMode,
        audience: Audience,
    ) -> StoreResult<EmailKindAudience> {
        let audience_json = serde_json::to_value(&audience)
            .map_err(|e| StoreError::SerdeError("Failed to serialize audience".into(), e))?;
        self.transaction(|conn| {
            let actor = &actor;
            let audience = &audience;
            let audience_json = &audience_json;
            async move {
                self.internal
                    .validate_kind_audience(conn, tenant_id, invoicing_entity_id, kind, audience)
                    .await?;
                let before = EmailKindAudienceRow::find(
                    conn,
                    tenant_id,
                    invoicing_entity_id,
                    &kind.to_string(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                let upserted = EmailKindAudienceRow {
                    invoicing_entity_id,
                    tenant_id,
                    kind: kind.to_string(),
                    mode: mode.into(),
                    audience: audience_json.clone(),
                    updated_at: common_domain::clock::now().naive_utc(),
                }
                .upsert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                self.internal
                    .audit_kind_audience(
                        conn,
                        tenant_id,
                        actor,
                        invoicing_entity_id,
                        kind,
                        before.as_ref(),
                        Some(&upserted),
                    )
                    .await?;
                let stored: EmailKindAudience = upserted.try_into()?;
                Ok(stored)
            }
            .scope_boxed()
        })
        .await
    }

    async fn clear_email_kind_audience(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
    ) -> StoreResult<()> {
        if !kind.takes_audience_rule() {
            return Err(invalid(
                "Payment reminders target audiences through their reminder sequences",
            ));
        }
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                self.internal
                    .entity_in_tenant(conn, tenant_id, invoicing_entity_id)
                    .await?;
                let removed = EmailKindAudienceRow::delete(
                    conn,
                    tenant_id,
                    invoicing_entity_id,
                    &kind.to_string(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                if let Some(removed) = removed {
                    self.internal
                        .audit_kind_audience(
                            conn,
                            tenant_id,
                            actor,
                            invoicing_entity_id,
                            kind,
                            Some(&removed),
                            None,
                        )
                        .await?;
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn preview_email_kind_audience(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        mode: EmailAudienceMode,
        audience: &Audience,
    ) -> StoreResult<EmailKindAudiencePreview> {
        let mut conn = self.get_conn().await?;
        self.internal
            .validate_kind_audience(&mut conn, tenant_id, invoicing_entity_id, kind, audience)
            .await?;
        let compiled = compile(audience)?;

        if kind.customer_level_audience() {
            let ids = list_customer_ids_for_entity(
                &mut conn,
                tenant_id,
                invoicing_entity_id,
                AUDIENCE_PREVIEW_SCAN_LIMIT,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            let matched = evaluate_customer_audience(&mut conn, tenant_id, &ids, &compiled)
                .await?
                .len();
            let receiving = match mode {
                EmailAudienceMode::Exclude => ids.len() - matched,
                EmailAudienceMode::Only => matched,
            };
            return Ok(EmailKindAudiencePreview {
                scanned_customers: ids.len(),
                matched_customers: matched,
                receiving_customers: receiving,
                ..Default::default()
            });
        }

        let ids = list_recent_invoice_ids_for_entity(
            &mut conn,
            tenant_id,
            invoicing_entity_id,
            AUDIENCE_PREVIEW_SCAN_LIMIT,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let matched = evaluate_audience(&mut conn, tenant_id, &ids, &compiled).await?;
        let matched_set: HashSet<InvoiceId> = matched.iter().copied().collect();
        let receiving: Vec<InvoiceId> = ids
            .iter()
            .copied()
            .filter(|id| !mode.suppresses(matched_set.contains(id)))
            .collect();
        let scanned_customers = distinct_customers(&mut conn, tenant_id, &ids).await?;
        let matched_customers = distinct_customers(&mut conn, tenant_id, &matched).await?;
        let receiving_customers = distinct_customers(&mut conn, tenant_id, &receiving).await?;
        Ok(EmailKindAudiencePreview {
            scanned_invoices: ids.len(),
            scanned_customers,
            matched_invoices: matched.len(),
            matched_customers,
            receiving_invoices: receiving.len(),
            receiving_customers,
        })
    }

    async fn email_audience_suppresses(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: EmailKind,
        subject: AudienceSubject,
    ) -> StoreResult<bool> {
        if !kind.takes_audience_rule() {
            return Ok(false);
        }
        let mut conn = self.get_conn().await?;
        let Some(row) = EmailKindAudienceRow::find(
            &mut conn,
            tenant_id,
            invoicing_entity_id,
            &kind.to_string(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(false);
        };
        let Some((rule, compiled)) = usable_rule(row) else {
            return Ok(false);
        };
        let matched = match subject {
            AudienceSubject::Invoice(id) => {
                !evaluate_audience(&mut conn, tenant_id, &[id], &compiled)
                    .await?
                    .is_empty()
            }
            AudienceSubject::Customer(id) => {
                !evaluate_customer_audience(&mut conn, tenant_id, &[id], &compiled)
                    .await?
                    .is_empty()
            }
        };
        Ok(rule.mode.suppresses(matched))
    }

    async fn resend_email(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        sent_email_id: EntityActivityId,
        request_id: &str,
    ) -> StoreResult<()> {
        let request_id = request_id.trim();
        if request_id.is_empty() || request_id.len() > 64 {
            return Err(invalid("request_id must be 1..=64 characters"));
        }
        let sent = self.get_sent_email_detail(tenant_id, sent_email_id).await?;
        let kind = sent.kind.ok_or_else(|| {
            Report::new(StoreError::FailedPrecondition(
                "This email predates the platform and cannot be resent".to_string(),
            ))
        })?;
        let mut context = sent.resend_context().ok_or_else(|| {
            Report::new(StoreError::FailedPrecondition(
                "This email has no re-renderable context".to_string(),
            ))
        })?;
        // An explicit resend is a manual send: it carries the row's current sequence and
        // generation so a restart in between does not drop it as superseded.
        if let EmailContext::PaymentReminder {
            invoice_id,
            sequence_id,
            generation,
            manual,
            ..
        } = &mut context
        {
            let mut conn = self.get_conn().await?;
            if let Some(row) =
                InvoicePaymentReminderRow::find_by_invoice_id(&mut conn, tenant_id, *invoice_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
            {
                *sequence_id = row.sequence_id.unwrap_or(*sequence_id);
                *generation = row.generation;
            }
            *manual = true;
        }

        self.transaction(|conn| {
            let actor = &actor;
            let context = context.clone();
            async move {
                let (entity_type, entity_id, invoicing_entity_id, customer_id) = match &context {
                    EmailContext::CreditNote { credit_note_id } => {
                        let cn = diesel_models::credit_notes::CreditNoteRow::find_by_id(
                            conn,
                            tenant_id,
                            *credit_note_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        (
                            EntityType::CreditNote,
                            credit_note_id.as_uuid(),
                            cn.invoicing_entity_id,
                            cn.customer_id,
                        )
                    }
                    EmailContext::WalletBalanceLow { customer_id, .. }
                    | EmailContext::AutoTopUpFailed { customer_id, .. }
                    | EmailContext::BudgetAlert { customer_id, .. } => {
                        let customer = CustomerRow::find_by_id(conn, customer_id, &tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        (
                            EntityType::Customer,
                            customer_id.as_uuid(),
                            customer.invoicing_entity_id,
                            *customer_id,
                        )
                    }
                    other => {
                        let invoice_id = other.invoice_id().ok_or_else(|| {
                            Report::new(StoreError::FailedPrecondition(
                                "email context has no invoice".to_string(),
                            ))
                        })?;
                        let invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        if kind == EmailKind::PaymentReminder
                            && !(invoice.status == InvoiceStatusEnum::Finalized
                                && invoice.amount_due > 0
                                && matches!(
                                    invoice.payment_status,
                                    InvoicePaymentStatus::Unpaid
                                        | InvoicePaymentStatus::PartiallyPaid
                                        | InvoicePaymentStatus::Errored
                                ))
                        {
                            return Err(Report::new(StoreError::FailedPrecondition(
                                "The invoice is no longer payable".to_string(),
                            )));
                        }
                        (
                            EntityType::Invoice,
                            invoice_id.as_uuid(),
                            invoice.invoicing_entity_id,
                            invoice.customer_id,
                        )
                    }
                };
                let settings = self
                    .internal
                    .get_email_settings_tx(conn, tenant_id, invoicing_entity_id)
                    .await?;
                if !settings.is_enabled(kind) {
                    return Err(Report::new(StoreError::FailedPrecondition(format!(
                        "{kind} emails are disabled for this invoicing entity"
                    ))));
                }
                let dedupe_key = format!("resend:{}:{}", sent_email_id.as_base62(), request_id);
                let msg = customer_email_message(
                    tenant_id,
                    kind,
                    context.clone(),
                    dedupe_key,
                    Some(sent_email_id),
                )?;
                self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, vec![msg])
                    .await?;
                self.internal
                    .record_audit_tx(
                        conn,
                        tenant_id,
                        actor,
                        AuditInput::Activity(
                            Activity::new(ActivityType::EmailResent, entity_type, entity_id)
                                .with_metadata(serde_json::json!({
                                    "kind": kind,
                                    "sent_email_id": sent_email_id.as_base62(),
                                    "request_id": request_id,
                                }))
                                .agg_customer(customer_id),
                        ),
                    )
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn resend_unknown_email(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        activity_id: EntityActivityId,
        request_id: &str,
    ) -> StoreResult<()> {
        let request_id = request_id.trim();
        if request_id.is_empty() || request_id.len() > 64 {
            return Err(invalid("request_id must be 1..=64 characters"));
        }
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let entry = EntityActivityRow::find_by_id_for_update(conn, tenant_id, activity_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(format!(
                            "activity {} not found",
                            activity_id.as_base62()
                        )))
                    })?;
                if entry.activity_type != ActivityType::EmailDeliveryUnknown.to_string() {
                    return Err(failed_precondition(
                        "Only an email whose delivery is unknown can be sent again",
                    ));
                }
                let metadata = entry.metadata.clone().unwrap_or_default();
                let kind = parse_kind(metadata.get("kind").and_then(|k| k.as_str()))
                    .ok_or_else(|| failed_precondition("This entry names no email kind"))?;
                if kind == EmailKind::PaymentReminder {
                    return Err(failed_precondition(
                        "Send payment reminders again from the invoice's reminders card",
                    ));
                }
                if already_requested(conn, tenant_id, &entry, activity_id, request_id).await? {
                    return Ok(());
                }
                let context = unknown_email_context(conn, tenant_id, kind, &entry).await?;

                // Anything recorded since the lost claim was won, including the lost send itself.
                let claimed_at = match metadata.get("dedupe_key").and_then(|k| k.as_str()) {
                    Some(key) => EmailSendClaimRow::find_by_dedupe_key(conn, key)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .filter(|c| c.tenant_id == tenant_id)
                        .map(|c| c.claimed_at),
                    None => None,
                };
                let since = claimed_at.map_or(entry.occurred_at, |c| c.min(entry.occurred_at));
                let context_json = serde_json::to_value(&context).map_err(|e| {
                    StoreError::SerdeError("Failed to serialize email context".into(), e)
                })?;
                if SentEmailRow::exists_for_context_since(
                    conn,
                    tenant_id,
                    &kind.to_string(),
                    context_json,
                    since,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                {
                    return Err(failed_precondition(
                        "This email has been sent since; resend that copy from its preview instead",
                    ));
                }

                let invoicing_entity_id = match &context {
                    EmailContext::CreditNote { credit_note_id } => {
                        diesel_models::credit_notes::CreditNoteRow::find_by_id(
                            conn,
                            tenant_id,
                            *credit_note_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .invoicing_entity_id
                    }
                    EmailContext::WalletBalanceLow { customer_id, .. }
                    | EmailContext::AutoTopUpFailed { customer_id, .. }
                    | EmailContext::BudgetAlert { customer_id, .. } => {
                        CustomerRow::find_by_id(conn, customer_id, &tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .invoicing_entity_id
                    }
                    other => {
                        let invoice_id = other
                            .invoice_id()
                            .ok_or_else(|| failed_precondition("email context has no invoice"))?;
                        InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .invoicing_entity_id
                    }
                };
                let settings = self
                    .internal
                    .get_email_settings_tx(conn, tenant_id, invoicing_entity_id)
                    .await?;
                if !settings.is_enabled(kind) {
                    return Err(failed_precondition(format!(
                        "{kind} emails are disabled for this invoicing entity"
                    )));
                }

                // One key per entry, not per click: two clicks racing the first send collapse on it.
                let dedupe_key = format!("{RESEND_UNKNOWN_KEY_PREFIX}:{}", activity_id.as_base62());
                // An operator's send: manual origin, so the kind's audience rule doesn't drop it.
                let msg: PgmqMessageNew = SendEmailRequest::CustomerEmail {
                    tenant_id,
                    kind,
                    context,
                    dedupe_key,
                    resent_from: None,
                    manual: true,
                    attempts: Default::default(),
                    recipient: None,
                }
                .try_into()?;
                self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, vec![msg])
                    .await?;
                let entity_type = EntityType::from_str(&entry.entity_type).map_err(|_| {
                    failed_precondition(format!("unknown entity type {}", entry.entity_type))
                })?;
                let mut audit =
                    Activity::new(ActivityType::EmailResent, entity_type, entry.entity_id)
                        .with_metadata(serde_json::json!({
                            "kind": kind,
                            "delivery_unknown_activity_id": activity_id.as_base62(),
                            "request_id": request_id,
                        }));
                if let Some(customer) = entry.agg_customer_id {
                    audit = audit.agg_customer(CustomerId::from(customer));
                }
                if let Some(subscription) = entry.agg_subscription_id {
                    audit = audit
                        .agg_subscription(common_domain::ids::SubscriptionId::from(subscription));
                }
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(audit))
                    .await
            }
            .scope_boxed()
        })
        .await
    }
}

/// Key prefix of an operator's re-send of a delivery-unknown email.
const RESEND_UNKNOWN_KEY_PREFIX: &str = "resend_unknown";
/// How many chained delivery-unknown re-sends are followed back to the email they recover.
const MAX_UNKNOWN_CHAIN: usize = 8;

fn failed_precondition(msg: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::FailedPrecondition(msg.into()))
}

/// Whether this `request_id` already re-sent this delivery-unknown entry (a retried click).
async fn already_requested(
    conn: &mut PgConn,
    tenant_id: TenantId,
    entry: &EntityActivityRow,
    activity_id: EntityActivityId,
    request_id: &str,
) -> StoreResult<bool> {
    let activity_id = activity_id.as_base62();
    let resent = EntityActivityRow::list_by_entity_and_type(
        conn,
        tenant_id,
        &entry.entity_type,
        entry.entity_id,
        &ActivityType::EmailResent.to_string(),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(resent.iter().any(|a| {
        a.metadata.as_ref().is_some_and(|m| {
            m.get("delivery_unknown_activity_id")
                .and_then(|v| v.as_str())
                == Some(activity_id.as_str())
                && m.get("request_id").and_then(|v| v.as_str()) == Some(request_id)
        })
    }))
}

/// The context a delivery-unknown entry was sent with: its `context` metadata when recorded,
/// else derived from its entity and dedupe key (`{kind}:{payment_transaction_id}`, a resend of
/// a sent email, or an earlier re-send of a delivery-unknown entry).
async fn unknown_email_context(
    conn: &mut PgConn,
    tenant_id: TenantId,
    kind: EmailKind,
    entry: &EntityActivityRow,
) -> StoreResult<EmailContext> {
    let cannot_rebuild = || failed_precondition("This email cannot be rebuilt from its activity");
    let (entity_type, entity_id) = (entry.entity_type.clone(), entry.entity_id);
    let on_invoice = entity_type == EntityType::Invoice.to_string();
    let mut metadata = entry.metadata.clone().unwrap_or_default();
    for _ in 0..MAX_UNKNOWN_CHAIN {
        if let Some(context) = metadata.get("context").filter(|c| !c.is_null()) {
            return serde_json::from_value(context.clone()).map_err(|e| {
                Report::new(StoreError::SerdeError(
                    "Failed to deserialize email context".into(),
                    e,
                ))
            });
        }
        match kind {
            EmailKind::InvoiceReady | EmailKind::InvoicePaid if on_invoice => {
                return Ok(EmailContext::Invoice {
                    invoice_id: InvoiceId::from(entity_id),
                });
            }
            EmailKind::CreditNoteIssued if entity_type == EntityType::CreditNote.to_string() => {
                return Ok(EmailContext::CreditNote {
                    credit_note_id: common_domain::ids::CreditNoteId::from(entity_id),
                });
            }
            EmailKind::PaymentFailed | EmailKind::AuthenticationRequired if on_invoice => {}
            _ => return Err(cannot_rebuild()),
        }

        let key = metadata
            .get("dedupe_key")
            .and_then(|k| k.as_str())
            .unwrap_or_default()
            .to_string();
        if let Some(tx) = key.strip_prefix(&format!("{kind}:")) {
            let payment_transaction_id = common_domain::ids::PaymentTransactionId::from_str(tx)
                .map_err(|_| cannot_rebuild())?;
            let invoice_id = InvoiceId::from(entity_id);
            return Ok(match kind {
                EmailKind::PaymentFailed => EmailContext::PaymentFailed {
                    invoice_id,
                    payment_transaction_id,
                },
                _ => EmailContext::AuthenticationRequired {
                    invoice_id,
                    payment_transaction_id,
                },
            });
        }
        let mut parts = key.splitn(3, ':');
        let (prefix, id) = (parts.next(), parts.next());
        let id = id
            .and_then(|id| EntityActivityId::from_str(id).ok())
            .ok_or_else(cannot_rebuild)?;
        match prefix {
            Some("resend") => {
                let sent: SentEmailDetail =
                    SentEmailRow::find_with_entity_by_id(conn, tenant_id, id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .ok_or_else(cannot_rebuild)?
                        .try_into()?;
                return sent.resend_context().ok_or_else(cannot_rebuild);
            }
            Some(RESEND_UNKNOWN_KEY_PREFIX) => {
                metadata = EntityActivityRow::find_by_id_for_update(conn, tenant_id, id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .and_then(|a| a.metadata)
                    .ok_or_else(cannot_rebuild)?;
            }
            _ => return Err(cannot_rebuild()),
        }
    }
    Err(cannot_rebuild())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> EmailSettings {
        EmailSettings::defaults(InvoicingEntityId::new())
    }

    #[test]
    fn patch_rejects_bad_reply_to_sixth_bcc_and_bad_timezone() {
        let bad_reply = EmailSettingsPatch {
            reply_to: Some(Some("not an address".into())),
            ..Default::default()
        };
        assert!(apply_patch(defaults(), bad_reply).is_err());

        let six = EmailSettingsPatch {
            bcc: Some((0..6).map(|i| format!("audit{i}@acme.example")).collect()),
            ..Default::default()
        };
        assert!(apply_patch(defaults(), six).is_err());

        let tz = EmailSettingsPatch {
            timezone: Some(Some("Not/AZone".into())),
            ..Default::default()
        };
        assert!(apply_patch(defaults(), tz).is_err());

        let hour = EmailSettingsPatch {
            reminder_send_hour: Some(24),
            ..Default::default()
        };
        assert!(apply_patch(defaults(), hour).is_err());
    }

    #[test]
    fn patch_dedupes_bcc_and_clears_with_empty_strings() {
        let patched = apply_patch(
            EmailSettings {
                sender_name: Some("Acme".into()),
                ..defaults()
            },
            EmailSettingsPatch {
                sender_name: Some(None),
                bcc: Some(vec![
                    "Audit@acme.example".into(),
                    "audit@acme.example ".into(),
                    "".into(),
                ]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(patched.sender_name, None);
        assert_eq!(patched.bcc, vec!["audit@acme.example".to_string()]);
        assert_eq!(patched.disabled_kinds, EmailKind::default_disabled());
    }
}
