use crate::StoreResult;
use crate::domain::InvoicingEntity;
use crate::domain::customer_emails::{EmailContext, EmailKind};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::payment_reminders::{
    Audience, CustomerReminderPreference, EmailCopy, EmailCopyByLocale, InvoiceReminderState,
    MAX_OFFSET_DAYS, MAX_STEPS, MIN_OFFSET_DAYS, PaymentReminderSequence,
    PaymentReminderSequenceNew, PaymentReminderSequencePatch, PaymentReminderStepInput,
    SequenceRef,
};
use crate::errors::StoreError;
use crate::repositories::CustomerEmailsInterface;
use crate::services::customer_locale::customer_enabled_locales;
use crate::services::payment_reminders::audience::{
    CompiledPredicate, compile, evaluate_audience, validate_audience,
};
use crate::services::payment_reminders::schedule::{
    anchor_date, ceil_to_send_slot, fire_time, schedulable,
};
pub use crate::services::payment_reminders::state::SequenceResolutionRow;
use crate::services::payment_reminders::sweep::{
    is_payable, is_reminder_eligible, manual_reminder_dedupe_key,
};
use crate::store::{PgConn, Store, StoreInternal};
use chrono::NaiveDateTime;
use common_domain::ids::{
    BaseId, CustomerId, InvoiceId, InvoicingEntityId, PaymentReminderSequenceId,
    PaymentReminderStepId, TenantId,
};
use common_domain::lang;
use diesel_models::customers::CustomerRow;
use diesel_models::email_send_claim::EmailSendClaimRow;
use diesel_models::enums::{
    InvoicePaymentStatus, PaymentReminderStepActionEnum, ReminderStateEnum, ReminderStopReasonEnum,
};
use diesel_models::invoices::{InvoiceLockRow, InvoiceRow};
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::payment_reminders::{
    InvoicePaymentReminderRow, PaymentReminderSequenceRow, PaymentReminderSequenceRowNew,
    PaymentReminderStepRow, PaymentReminderStepRowNew,
};
use diesel_models::query::payment_reminders::{
    InvoiceReminderUpdates, PaymentReminderSequenceRowPatch, audience_preview_sample,
    count_distinct_customers, list_open_invoice_ids_for_entity, manual_reminder_recorded,
};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::{HashMap, HashSet};

pub const DEFAULT_SEQUENCE_NAME: &str = "Default reminders";
pub const MAX_NAME_CHARS: usize = 80;
/// Invoices an audience preview evaluates at most (newest first).
const PREVIEW_SCAN_LIMIT: i64 = 5_000;

#[derive(Debug, Clone, PartialEq)]
pub struct SequenceWithInFlight {
    pub sequence: PaymentReminderSequence,
    pub in_flight_invoices: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudiencePreviewRow {
    pub invoice_id: InvoiceId,
    pub invoice_number: String,
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub amount_due: i64,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudiencePreview {
    pub invoice_count: usize,
    pub customer_count: usize,
    /// Matched invoices a sequence positioned earlier would claim first.
    pub claimed_by_earlier: usize,
    pub sample: Vec<AudiencePreviewRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerReminderPreferenceView {
    pub preference: CustomerReminderPreference,
    /// Live non-default sequences of the customer's invoicing entity.
    pub available_sequences: Vec<SequenceRef>,
}

#[async_trait::async_trait]
pub trait PaymentRemindersInterface {
    /// Seeds the entity's default sequence first (idempotent, `Actor::System`, no audit).
    async fn list_reminder_sequences(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        include_archived: bool,
    ) -> StoreResult<Vec<SequenceWithInFlight>>;

    async fn get_reminder_sequence(
        &self,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
    ) -> StoreResult<SequenceWithInFlight>;

    /// The new sequence lands just before the default, which keeps the highest live position.
    async fn create_reminder_sequence(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        sequence: PaymentReminderSequenceNew,
    ) -> StoreResult<SequenceWithInFlight>;

    /// A `steps` replacement re-arms the frozen invoices in the same transaction.
    async fn update_reminder_sequence(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        patch: PaymentReminderSequencePatch,
        now: NaiveDateTime,
    ) -> StoreResult<SequenceWithInFlight>;

    /// `ordered_ids` = every live non-default sequence of the entity; the default stays last.
    async fn reorder_reminder_sequences(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        ordered_ids: Vec<PaymentReminderSequenceId>,
    ) -> StoreResult<Vec<SequenceWithInFlight>>;

    /// Refused for the default. Stops in-flight invoices and nulls customer overrides;
    /// returns the number of invoices stopped.
    async fn archive_reminder_sequence(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
        now: NaiveDateTime,
    ) -> StoreResult<usize>;

    /// Over the entity's open finalized invoices; `sequence_id` bounds "earlier" to the
    /// sequences positioned before it (absent = every live non-default sequence).
    async fn preview_audience(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        audience: &Audience,
        sequence_id: Option<PaymentReminderSequenceId>,
        sample_limit: usize,
    ) -> StoreResult<AudiencePreview>;

    async fn get_customer_reminder_preference(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<CustomerReminderPreferenceView>;

    async fn get_invoice_reminder_state(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState>;

    /// Paid hook: the schedule is cleared, the row kept as `ENDED`, unless the invoice
    /// (locked and re-read) is payable or processing again.
    async fn end_invoice_reminders(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()>;

    async fn preview_invoice_reminder_resolution(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<SequenceResolutionRow>>;

    /// `until` is ceiled to the entity's send slot; expiry catches up the last elapsed step.
    async fn pause_invoice_reminders(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        until: NaiveDateTime,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState>;

    async fn resume_invoice_reminders(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState>;

    async fn stop_invoice_reminders(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState>;

    /// Generation bumped, sequence re-resolved over enabled sequences (none ⇒
    /// `FailedPrecondition`), the last elapsed step executes once at the next slot.
    async fn restart_invoice_reminders(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState>;

    /// Needs a row with a frozen sequence (`FailedPrecondition("Restart reminders first")`
    /// otherwise); `offset_days` must be a step of it. Bypasses stop, pause and cooldown,
    /// honours opt-out and the kind toggle; `request_id` collapses double clicks.
    async fn send_invoice_reminder_now(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        offset_days: i32,
        request_id: &str,
        now: NaiveDateTime,
    ) -> StoreResult<()>;

    /// `sequence_id` must be a live sequence of the customer's entity; `opt_out = true` stops
    /// the customer's in-flight invoices in the same transaction.
    async fn set_customer_reminder_preference(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer_id: CustomerId,
        preference: CustomerReminderPreference,
        now: NaiveDateTime,
    ) -> StoreResult<CustomerReminderPreferenceView>;
}

fn invalid(msg: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(msg.into()))
}

fn not_found(id: PaymentReminderSequenceId) -> Report<StoreError> {
    Report::new(StoreError::ValueNotFound(format!(
        "Payment reminder sequence {} not found",
        id.as_base62()
    )))
}

fn sequence_ref(row: &PaymentReminderSequenceRow) -> SequenceRef {
    SequenceRef {
        id: row.id,
        name: row.name.clone(),
        enabled: row.enabled,
        archived: row.archived_at.is_some(),
    }
}

/// Locales step copy must cover: the entity's customer-facing ones, default first.
fn copy_locales(entity: InvoicingEntityRow) -> Vec<&'static str> {
    customer_enabled_locales(&InvoicingEntity::from(entity))
}

pub(crate) fn validate_name(name: &str) -> StoreResult<()> {
    let len = name.trim().chars().count();
    if len == 0 || len > MAX_NAME_CHARS {
        return Err(invalid(format!(
            "Sequence name must be 1 to {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(())
}

fn validate_copy(locale: &str, copy: &EmailCopy) -> StoreResult<()> {
    meteroid_mailer::copy::validate(&copy.subject, &copy.body)
        .map_err(|e| invalid(format!("Copy for {locale}: {e}")))
}

/// Keys are template locales, every enabled email locale has a non-blank entry, every
/// entry passes the operator-copy rules (variables, lengths, Markdown whitelist). Entries for
/// locales not enabled are kept.
pub(crate) fn validate_content(
    content: &EmailCopyByLocale,
    offset_days: i32,
    enabled_locales: &[&str],
) -> StoreResult<()> {
    // No entries = the built-in copy, which exists in every locale.
    if content.0.is_empty() {
        return Ok(());
    }
    for (locale, copy) in &content.0 {
        if !lang::SUPPORTED.contains(&locale.as_str()) {
            return Err(invalid(format!("Unsupported copy locale {locale}")));
        }
        validate_copy(locale, copy)?;
    }
    let missing: Vec<&str> = enabled_locales
        .iter()
        .copied()
        .filter(|locale| {
            !content
                .0
                .get(*locale)
                .is_some_and(|c| !c.subject.trim().is_empty() && !c.body.trim().is_empty())
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let step = if offset_days == 0 {
        "0".to_string()
    } else {
        format!("{offset_days:+}")
    };
    Err(invalid(format!(
        "Step {step}: copy missing for {}",
        missing.join(", ")
    )))
}

/// 0..=20 steps, offsets strictly increasing in `[-30, 365]`, copy as
/// [`validate_content`] checks it in every enabled email locale (default first).
pub(crate) fn validate_steps(
    steps: &[PaymentReminderStepInput],
    enabled_locales: &[&str],
) -> StoreResult<()> {
    if steps.len() > MAX_STEPS {
        return Err(invalid(format!("A sequence has at most {MAX_STEPS} steps")));
    }
    let mut previous: Option<i32> = None;
    for step in steps {
        if !(MIN_OFFSET_DAYS..=MAX_OFFSET_DAYS).contains(&step.offset_days) {
            return Err(invalid(format!(
                "Step offsets must be between {MIN_OFFSET_DAYS} and {MAX_OFFSET_DAYS} days"
            )));
        }
        if previous.is_some_and(|p| step.offset_days <= p) {
            return Err(invalid("Step offsets must be strictly increasing"));
        }
        previous = Some(step.offset_days);
        validate_content(&step.content, step.offset_days, enabled_locales)?;
    }
    Ok(())
}

fn audience_json(audience: &Audience) -> StoreResult<serde_json::Value> {
    serde_json::to_value(audience).map_err(|e| {
        Report::new(StoreError::SerdeError(
            "Failed to serialize audience".into(),
            e,
        ))
    })
}

fn step_rows(
    tenant_id: TenantId,
    sequence_id: PaymentReminderSequenceId,
    steps: &[PaymentReminderStepInput],
) -> StoreResult<Vec<PaymentReminderStepRowNew>> {
    steps
        .iter()
        .map(|s| {
            Ok(PaymentReminderStepRowNew {
                id: PaymentReminderStepId::new(),
                tenant_id,
                sequence_id,
                offset_days: s.offset_days,
                action: PaymentReminderStepActionEnum::Notify,
                content: serde_json::to_value(&s.content).map_err(|e| {
                    StoreError::SerdeError("Failed to serialize step content".into(), e)
                })?,
            })
        })
        .collect()
}

/// The seeded default ladder: no stored copy, so every step renders the built-in
/// reminder text for its offset, in whichever locale the email resolves to.
fn default_steps() -> Vec<PaymentReminderStepInput> {
    [-3, 0, 7, 14]
        .into_iter()
        .map(|offset_days| PaymentReminderStepInput {
            offset_days,
            content: EmailCopyByLocale::default(),
        })
        .collect()
}

impl StoreInternal {
    /// Under a per-entity xact lock, `INSERT … ON CONFLICT DO NOTHING` on the live-default index,
    /// then the −3/0/+7/+14 steps on built-in copy; unaudited, starts disabled.
    pub async fn ensure_default_sequence(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<()> {
        PaymentReminderSequenceRowNew::lock_entity_seed(conn, invoicing_entity_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let Some(id) = PaymentReminderSequenceRowNew::insert_default_if_absent(
            conn,
            PaymentReminderSequenceId::new(),
            tenant_id,
            invoicing_entity_id,
            DEFAULT_SEQUENCE_NAME,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(());
        };
        let rows = step_rows(tenant_id, id, &default_steps())?;
        PaymentReminderStepRowNew::insert_batch(&rows, conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    async fn load_sequences(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        rows: Vec<PaymentReminderSequenceRow>,
    ) -> StoreResult<Vec<SequenceWithInFlight>> {
        let ids: Vec<PaymentReminderSequenceId> = rows.iter().map(|r| r.id).collect();
        let mut steps_by_sequence: HashMap<PaymentReminderSequenceId, Vec<PaymentReminderStepRow>> =
            HashMap::new();
        for step in PaymentReminderStepRow::list_by_sequence_ids(conn, tenant_id, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            steps_by_sequence
                .entry(step.sequence_id)
                .or_default()
                .push(step);
        }
        let in_flight: HashMap<PaymentReminderSequenceId, i64> =
            InvoicePaymentReminderRow::count_in_flight_by_sequence(conn, tenant_id, &ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|r| (PaymentReminderSequenceId::from(r.sequence_id), r.in_flight))
                .collect();
        rows.into_iter()
            .map(|row| {
                let steps = steps_by_sequence.remove(&row.id).unwrap_or_default();
                let in_flight_invoices = in_flight.get(&row.id).copied().unwrap_or(0);
                Ok(SequenceWithInFlight {
                    sequence: PaymentReminderSequence::from_rows(row, steps)?,
                    in_flight_invoices,
                })
            })
            .collect()
    }

    async fn load_sequence(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
    ) -> StoreResult<SequenceWithInFlight> {
        let row = PaymentReminderSequenceRow::find_by_id(conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| not_found(id))?;
        self.load_sequences(conn, tenant_id, vec![row])
            .await?
            .pop()
            .ok_or_else(|| not_found(id))
    }

    async fn entity_row(
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

    async fn audit_sequence(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        actor: &Actor,
        activity_type: ActivityType,
        id: PaymentReminderSequenceId,
        metadata: serde_json::Value,
    ) -> StoreResult<()> {
        let activity = Activity::new(
            activity_type,
            EntityType::PaymentReminderSequence,
            id.as_uuid(),
        )
        .with_metadata(metadata);
        self.record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
            .await
    }

    async fn audit_customer_changes(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        actor: &Actor,
        customer_id: CustomerId,
        changes: Vec<serde_json::Value>,
    ) -> StoreResult<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let activity = Activity::new(
            ActivityType::CustomerUpdated,
            EntityType::Customer,
            customer_id.as_uuid(),
        )
        .with_metadata(serde_json::json!({ "changes": changes }));
        self.record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
            .await
    }

    /// Matches of each earlier live enabled non-default sequence, unioned.
    async fn claimed_by_earlier(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        earlier: &[PaymentReminderSequenceRow],
        ids: &[InvoiceId],
    ) -> StoreResult<HashSet<InvoiceId>> {
        let mut claimed = HashSet::new();
        for row in earlier {
            let Some(audience) = row.audience.clone() else {
                continue;
            };
            let audience: Audience = serde_json::from_value(audience).map_err(|e| {
                StoreError::SerdeError("Failed to deserialize sequence audience".into(), e)
            })?;
            let compiled: CompiledPredicate = compile(&audience)?;
            claimed.extend(evaluate_audience(conn, tenant_id, ids, &compiled).await?);
        }
        Ok(claimed)
    }
}

#[async_trait::async_trait]
impl PaymentRemindersInterface for Store {
    async fn list_reminder_sequences(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        include_archived: bool,
    ) -> StoreResult<Vec<SequenceWithInFlight>> {
        let mut conn = self.get_conn().await?;
        self.internal
            .entity_row(&mut conn, tenant_id, invoicing_entity_id)
            .await?;
        let mut rows = PaymentReminderSequenceRow::list_by_entity(
            &mut conn,
            tenant_id,
            invoicing_entity_id,
            include_archived,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        if !rows
            .iter()
            .any(|r| r.audience.is_none() && r.archived_at.is_none())
        {
            // The seed writes the sequence and its steps atomically (a half-seeded default
            // would silently send nothing); the read connection is released meanwhile.
            drop(conn);
            self.transaction(|conn| {
                async move {
                    self.internal
                        .ensure_default_sequence(conn, tenant_id, invoicing_entity_id)
                        .await
                }
                .scope_boxed()
            })
            .await?;
            conn = self.get_conn().await?;
            rows = PaymentReminderSequenceRow::list_by_entity(
                &mut conn,
                tenant_id,
                invoicing_entity_id,
                include_archived,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        }
        self.internal
            .load_sequences(&mut conn, tenant_id, rows)
            .await
    }

    async fn get_reminder_sequence(
        &self,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
    ) -> StoreResult<SequenceWithInFlight> {
        let mut conn = self.get_conn().await?;
        self.internal.load_sequence(&mut conn, tenant_id, id).await
    }

    async fn create_reminder_sequence(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        sequence: PaymentReminderSequenceNew,
    ) -> StoreResult<SequenceWithInFlight> {
        validate_name(&sequence.name)?;
        let Some(audience) = sequence.audience.as_ref() else {
            return Err(invalid(
                "An audience is required; the default sequence is created automatically",
            ));
        };
        let audience_value = audience_json(audience)?;

        let id = self
            .transaction(|conn| {
                let actor = &actor;
                let sequence = &sequence;
                async move {
                    let entity = self
                        .internal
                        .entity_row(conn, tenant_id, sequence.invoicing_entity_id)
                        .await?;
                    validate_steps(&sequence.steps, &copy_locales(entity))?;
                    validate_audience(conn, tenant_id, sequence.invoicing_entity_id, audience)
                        .await?;
                    self.internal
                        .ensure_default_sequence(conn, tenant_id, sequence.invoicing_entity_id)
                        .await?;
                    let default = PaymentReminderSequenceRow::find_default_for_update(
                        conn,
                        tenant_id,
                        sequence.invoicing_entity_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(
                            "Default payment reminder sequence not found".into(),
                        ))
                    })?;

                    // The default parks on a negative position while the new row takes its slot.
                    let position = default.position;
                    PaymentReminderSequenceRow::set_position(
                        conn,
                        tenant_id,
                        default.id,
                        -position - 1,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    let row = PaymentReminderSequenceRowNew {
                        id: PaymentReminderSequenceId::new(),
                        tenant_id,
                        invoicing_entity_id: sequence.invoicing_entity_id,
                        name: sequence.name.trim().to_string(),
                        position,
                        enabled: sequence.enabled,
                        audience: Some(audience_value.clone()),
                    }
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    PaymentReminderSequenceRow::set_position(
                        conn,
                        tenant_id,
                        default.id,
                        position + 1,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let rows = step_rows(tenant_id, row.id, &sequence.steps)?;
                    PaymentReminderStepRowNew::insert_batch(&rows, conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    self.internal
                        .audit_sequence(
                            conn,
                            tenant_id,
                            actor,
                            ActivityType::ReminderSequenceCreated,
                            row.id,
                            serde_json::json!({
                                "name": row.name,
                                "enabled": row.enabled,
                                "steps": sequence.steps.len(),
                            }),
                        )
                        .await?;
                    Ok(row.id)
                }
                .scope_boxed()
            })
            .await?;

        self.get_reminder_sequence(tenant_id, id).await
    }

    async fn update_reminder_sequence(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        patch: PaymentReminderSequencePatch,
        now: NaiveDateTime,
    ) -> StoreResult<SequenceWithInFlight> {
        if let Some(name) = &patch.name {
            validate_name(name)?;
        }
        let id = patch.id;

        self.transaction(|conn| {
            let actor = &actor;
            let patch = &patch;
            async move {
                let current =
                    PaymentReminderSequenceRow::find_by_id_for_update(conn, tenant_id, id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .ok_or_else(|| not_found(id))?;
                if current.archived_at.is_some() {
                    return Err(Report::new(StoreError::FailedPrecondition(
                        "An archived sequence cannot be edited".into(),
                    )));
                }
                let is_default = current.audience.is_none();
                let entity = self
                    .internal
                    .entity_row(conn, tenant_id, current.invoicing_entity_id)
                    .await?;

                let mut changes: Vec<&str> = Vec::new();
                let audience = match &patch.audience {
                    None => None,
                    Some(None) if is_default => Some(None),
                    Some(None) => {
                        return Err(invalid("Only the default sequence has no audience"));
                    }
                    Some(Some(_)) if is_default => {
                        return Err(invalid("The default sequence cannot receive an audience"));
                    }
                    Some(Some(audience)) => {
                        validate_audience(conn, tenant_id, current.invoicing_entity_id, audience)
                            .await?;
                        changes.push("audience");
                        Some(Some(audience_json(audience)?))
                    }
                };
                if let Some(steps) = &patch.steps {
                    validate_steps(steps, &copy_locales(entity))?;
                    changes.push("steps");
                }
                if patch
                    .name
                    .as_ref()
                    .is_some_and(|n| n.trim() != current.name)
                {
                    changes.push("name");
                }
                if patch.enabled.is_some_and(|e| e != current.enabled) {
                    changes.push("enabled");
                }

                PaymentReminderSequenceRow::patch(
                    conn,
                    tenant_id,
                    id,
                    PaymentReminderSequenceRowPatch {
                        name: patch.name.as_ref().map(|n| n.trim().to_string()),
                        enabled: patch.enabled,
                        audience,
                        updated_at: Some(now),
                    },
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                if let Some(steps) = &patch.steps {
                    PaymentReminderStepRow::delete_by_sequence(conn, tenant_id, id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let rows = step_rows(tenant_id, id, steps)?;
                    PaymentReminderStepRowNew::insert_batch(&rows, conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let max_offset = steps.iter().map(|s| s.offset_days).max();
                    InvoiceReminderUpdates::rearm_for_sequence(
                        conn, tenant_id, id, now, max_offset,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                }

                if !changes.is_empty() {
                    self.internal
                        .audit_sequence(
                            conn,
                            tenant_id,
                            actor,
                            ActivityType::ReminderSequenceUpdated,
                            id,
                            serde_json::json!({
                                "changes": changes
                                    .iter()
                                    .map(|f| serde_json::json!({ "field": f }))
                                    .collect::<Vec<_>>(),
                            }),
                        )
                        .await?;
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await?;

        self.get_reminder_sequence(tenant_id, id).await
    }

    async fn reorder_reminder_sequences(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        ordered_ids: Vec<PaymentReminderSequenceId>,
    ) -> StoreResult<Vec<SequenceWithInFlight>> {
        self.transaction(|conn| {
            let actor = &actor;
            let ordered_ids = &ordered_ids;
            async move {
                self.internal
                    .entity_row(conn, tenant_id, invoicing_entity_id)
                    .await?;
                self.internal
                    .ensure_default_sequence(conn, tenant_id, invoicing_entity_id)
                    .await?;
                let live = PaymentReminderSequenceRow::list_by_entity(
                    conn,
                    tenant_id,
                    invoicing_entity_id,
                    false,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                let (defaults, others): (Vec<_>, Vec<_>) =
                    live.into_iter().partition(|r| r.audience.is_none());
                let expected: HashSet<PaymentReminderSequenceId> =
                    others.iter().map(|r| r.id).collect();
                let given: HashSet<PaymentReminderSequenceId> = ordered_ids.iter().copied().collect();
                if given.len() != ordered_ids.len() || given != expected {
                    return Err(invalid(
                        "Reorder must list every live non-default sequence of the entity exactly once",
                    ));
                }

                let before: HashMap<PaymentReminderSequenceId, i32> = others
                    .iter()
                    .chain(defaults.iter())
                    .map(|r| (r.id, r.position))
                    .collect();
                let final_positions: Vec<(PaymentReminderSequenceId, i32)> = ordered_ids
                    .iter()
                    .copied()
                    .chain(defaults.iter().map(|r| r.id))
                    .enumerate()
                    .map(|(i, id)| (id, i as i32))
                    .collect();

                PaymentReminderSequenceRow::negate_live_positions(conn, tenant_id, invoicing_entity_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                for (id, position) in &final_positions {
                    PaymentReminderSequenceRow::set_position(conn, tenant_id, *id, *position)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }
                for (id, position) in &final_positions {
                    let from = before.get(id).copied();
                    if from == Some(*position) {
                        continue;
                    }
                    self.internal
                        .audit_sequence(
                            conn,
                            tenant_id,
                            actor,
                            ActivityType::ReminderSequenceUpdated,
                            *id,
                            serde_json::json!({
                                "changes": [{ "field": "position", "from": from, "to": position }],
                            }),
                        )
                        .await?;
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await?;

        self.list_reminder_sequences(tenant_id, invoicing_entity_id, false)
            .await
    }

    async fn archive_reminder_sequence(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
        now: NaiveDateTime,
    ) -> StoreResult<usize> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let current =
                    PaymentReminderSequenceRow::find_by_id_for_update(conn, tenant_id, id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .ok_or_else(|| not_found(id))?;
                if current.audience.is_none() {
                    return Err(invalid("The default sequence cannot be archived"));
                }
                if current.archived_at.is_some() {
                    return Err(Report::new(StoreError::FailedPrecondition(
                        "This sequence is already archived".into(),
                    )));
                }
                PaymentReminderSequenceRow::archive(conn, tenant_id, id, now)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                // Sequence, then the customers whose rows it writes in id order, as the sends and
                // payments (customer, invoice, reminder) and the overrides (sequence, customer).
                let mut locks = Vec::new();
                for customer_id in PaymentReminderSequenceRow::customers_of(conn, tenant_id, id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                {
                    locks.push(
                        CustomerRow::lock(conn, tenant_id, customer_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?,
                    );
                }
                let stopped = InvoiceReminderUpdates::stop_all_for_sequence(
                    conn,
                    &locks,
                    tenant_id,
                    id,
                    ReminderStopReasonEnum::SequenceArchived,
                    now,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                let customers = PaymentReminderSequenceRow::clear_customer_overrides(
                    conn, &locks, tenant_id, id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                for customer_id in &customers {
                    self.internal
                        .audit_customer_changes(
                            conn,
                            tenant_id,
                            actor,
                            *customer_id,
                            vec![serde_json::json!({
                                "field": "payment_reminder_sequence_id",
                                "from": id.as_base62(),
                                "to": null,
                            })],
                        )
                        .await?;
                }
                self.internal
                    .audit_sequence(
                        conn,
                        tenant_id,
                        actor,
                        ActivityType::ReminderSequenceArchived,
                        id,
                        serde_json::json!({
                            "stopped_invoices": stopped,
                            "cleared_customers": customers.len(),
                        }),
                    )
                    .await?;
                Ok(stopped)
            }
            .scope_boxed()
        })
        .await
    }

    async fn preview_audience(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        audience: &Audience,
        sequence_id: Option<PaymentReminderSequenceId>,
        sample_limit: usize,
    ) -> StoreResult<AudiencePreview> {
        let mut conn = self.get_conn().await?;
        validate_audience(&mut conn, tenant_id, invoicing_entity_id, audience).await?;
        let compiled = compile(audience)?;

        let ids = list_open_invoice_ids_for_entity(
            &mut conn,
            tenant_id,
            invoicing_entity_id,
            PREVIEW_SCAN_LIMIT,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let matched = evaluate_audience(&mut conn, tenant_id, &ids, &compiled).await?;

        let live = PaymentReminderSequenceRow::list_by_entity(
            &mut conn,
            tenant_id,
            invoicing_entity_id,
            false,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let own_position = sequence_id
            .and_then(|id| live.iter().find(|r| r.id == id))
            .map(|r| r.position);
        let earlier: Vec<PaymentReminderSequenceRow> = live
            .into_iter()
            .filter(|r| r.enabled && r.audience.is_some() && Some(r.id) != sequence_id)
            .filter(|r| own_position.is_none_or(|p| r.position < p))
            .collect();
        let claimed = self
            .internal
            .claimed_by_earlier(&mut conn, tenant_id, &earlier, &matched)
            .await?;

        let sample = audience_preview_sample(&mut conn, tenant_id, &matched, sample_limit as i64)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|r| AudiencePreviewRow {
                invoice_id: InvoiceId::from(r.invoice_id),
                invoice_number: r.invoice_number,
                customer_id: CustomerId::from(r.customer_id),
                customer_name: r.customer_name,
                amount_due: r.amount_due,
                currency: r.currency,
            })
            .collect();
        let customers = count_distinct_customers(&mut conn, tenant_id, &matched)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(AudiencePreview {
            invoice_count: matched.len(),
            customer_count: customers as usize,
            claimed_by_earlier: matched.iter().filter(|id| claimed.contains(id)).count(),
            sample,
        })
    }

    async fn get_customer_reminder_preference(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<CustomerReminderPreferenceView> {
        let mut conn = self.get_conn().await?;
        let customer = CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let available = PaymentReminderSequenceRow::list_by_entity(
            &mut conn,
            tenant_id,
            customer.invoicing_entity_id,
            false,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .iter()
        .filter(|r| r.audience.is_some())
        .map(sequence_ref)
        .collect();
        Ok(CustomerReminderPreferenceView {
            preference: CustomerReminderPreference {
                opt_out: customer.payment_reminders_opt_out,
                sequence_id: customer.payment_reminder_sequence_id,
            },
            available_sequences: available,
        })
    }

    async fn set_customer_reminder_preference(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer_id: CustomerId,
        preference: CustomerReminderPreference,
        now: NaiveDateTime,
    ) -> StoreResult<CustomerReminderPreferenceView> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                // Sequence before customer, the order archive takes: a concurrent archive
                // either sees this override or is seen here.
                let sequence = match preference.sequence_id {
                    Some(sequence_id) => Some(
                        PaymentReminderSequenceRow::find_by_id_for_update(
                            conn,
                            tenant_id,
                            sequence_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .ok_or_else(|| not_found(sequence_id))?,
                    ),
                    None => None,
                };
                let (_, customer) = CustomerRow::select_for_update(conn, customer_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if let Some(sequence) = sequence {
                    if sequence.invoicing_entity_id != customer.invoicing_entity_id {
                        return Err(invalid(
                            "The sequence belongs to another invoicing entity than the customer",
                        ));
                    }
                    if sequence.archived_at.is_some() {
                        return Err(invalid(
                            "An archived sequence cannot be a customer override",
                        ));
                    }
                    if sequence.audience.is_none() {
                        return Err(invalid(
                            "The default sequence cannot be a customer override",
                        ));
                    }
                }
                CustomerRow::set_payment_reminder_preference(
                    conn,
                    tenant_id,
                    customer_id,
                    preference.opt_out,
                    preference.sequence_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                if preference.opt_out {
                    InvoiceReminderUpdates::stop_all_for_customer(
                        conn,
                        tenant_id,
                        customer_id,
                        ReminderStopReasonEnum::CustomerOptOut,
                        now,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                }

                let mut changes = Vec::new();
                if customer.payment_reminders_opt_out != preference.opt_out {
                    changes.push(serde_json::json!({
                        "field": "payment_reminders_opt_out",
                        "from": customer.payment_reminders_opt_out,
                        "to": preference.opt_out,
                    }));
                }
                if customer.payment_reminder_sequence_id != preference.sequence_id {
                    changes.push(serde_json::json!({
                        "field": "payment_reminder_sequence_id",
                        "from": customer.payment_reminder_sequence_id.map(|id| id.as_base62()),
                        "to": preference.sequence_id.map(|id| id.as_base62()),
                    }));
                }
                self.internal
                    .audit_customer_changes(conn, tenant_id, actor, customer_id, changes)
                    .await
            }
            .scope_boxed()
        })
        .await?;

        self.get_customer_reminder_preference(tenant_id, customer_id)
            .await
    }

    // ---- per-invoice state and actions ------------------------------------------------------

    async fn get_invoice_reminder_state(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState> {
        let mut conn = self.get_conn().await?;
        self.internal
            .invoice_reminder_state_tx(&mut conn, tenant_id, invoice_id, now)
            .await
    }

    async fn end_invoice_reminders(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .invoice;
                // A late or redelivered paid event must not end a ladder a reopen restarted;
                // a pending payment holds the ladder, as in the sweep.
                if is_payable(&invoice)
                    || invoice.payment_status == InvoicePaymentStatus::Processing
                {
                    return Ok(());
                }
                InvoiceReminderUpdates::clear_schedule(
                    conn,
                    invoice_id,
                    tenant_id,
                    chrono::Utc::now().naive_utc(),
                )
                .await
                .map(|_| ())
                .map_err(Into::<Report<StoreError>>::into)
            }
            .scope_boxed()
        })
        .await
    }

    async fn preview_invoice_reminder_resolution(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<SequenceResolutionRow>> {
        let mut conn = self.get_conn().await?;
        self.internal
            .preview_invoice_reminder_resolution_tx(&mut conn, tenant_id, invoice_id)
            .await
    }

    async fn pause_invoice_reminders(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        until: NaiveDateTime,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState> {
        if until <= now {
            return Err(invalid("The pause must end in the future"));
        }
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let (invoice, row) = lock_invoice_and_reminder(conn, tenant_id, invoice_id).await?;
                active_row(row)?;
                let window = self
                    .internal
                    .get_email_settings_tx(conn, tenant_id, invoice.invoicing_entity_id)
                    .await?
                    .send_window();
                let until = ceil_to_send_slot(until, &window);
                InvoiceReminderUpdates::pause(conn, invoice_id, tenant_id, until)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                self.internal
                    .audit_invoice_reminder(
                        conn,
                        tenant_id,
                        actor,
                        ActivityType::PaymentReminderPaused,
                        &invoice,
                        serde_json::json!({ "until": until }),
                    )
                    .await?;
                self.internal
                    .invoice_reminder_state_tx(conn, tenant_id, invoice_id, now)
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn resume_invoice_reminders(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let (invoice, row) = lock_invoice_and_reminder(conn, tenant_id, invoice_id).await?;
                let row = active_row(row)?;
                if row.paused_until.is_some() {
                    InvoiceReminderUpdates::resume(conn, invoice_id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    self.internal
                        .audit_invoice_reminder(
                            conn,
                            tenant_id,
                            actor,
                            ActivityType::PaymentReminderResumed,
                            &invoice,
                            serde_json::json!({ "was_paused_until": row.paused_until }),
                        )
                        .await?;
                }
                self.internal
                    .invoice_reminder_state_tx(conn, tenant_id, invoice_id, now)
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn stop_invoice_reminders(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let (invoice, row) = lock_invoice_and_reminder(conn, tenant_id, invoice_id).await?;
                active_row(row)?;
                InvoiceReminderUpdates::stop(
                    conn,
                    invoice_id,
                    tenant_id,
                    ReminderStopReasonEnum::Operator,
                    now,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                self.internal
                    .audit_invoice_reminder(
                        conn,
                        tenant_id,
                        actor,
                        ActivityType::PaymentReminderStopped,
                        &invoice,
                        serde_json::json!({ "reason": "OPERATOR" }),
                    )
                    .await?;
                self.internal
                    .invoice_reminder_state_tx(conn, tenant_id, invoice_id, now)
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn restart_invoice_reminders(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let (invoice, row) = lock_invoice_and_reminder(conn, tenant_id, invoice_id).await?;
                if invoice.consolidated_into_invoice_id.is_some() {
                    return Err(failed_precondition(
                        "A consolidated child invoice is never reminded on its own",
                    ));
                }
                if !is_payable(&invoice) {
                    return Err(failed_precondition("This invoice is not open for payment"));
                }
                if !is_reminder_eligible(&invoice) {
                    return Err(failed_precondition(
                        "This invoice is collected automatically; reminders start once the charge fails",
                    ));
                }
                self.internal
                    .restart_reminders_tx(conn, tenant_id, &invoice, row.as_ref(), None, actor, now)
                    .await?;
                self.internal
                    .invoice_reminder_state_tx(conn, tenant_id, invoice_id, now)
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn send_invoice_reminder_now(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        offset_days: i32,
        request_id: &str,
        now: NaiveDateTime,
    ) -> StoreResult<()> {
        let request_id = request_id.trim();
        if request_id.is_empty() || request_id.len() > 64 {
            return Err(invalid("request_id must be 1..=64 characters"));
        }
        let dedupe_key = manual_reminder_dedupe_key(invoice_id, request_id);
        self.transaction(|conn| {
            let actor = &actor;
            let dedupe_key = &dedupe_key;
            async move {
                let (invoice, row) = lock_invoice_and_reminder(conn, tenant_id, invoice_id).await?;
                let sequence_id = row
                    .as_ref()
                    .filter(|r| r.state != ReminderStateEnum::NotEnrolled)
                    .and_then(|r| r.sequence_id)
                    .ok_or_else(|| failed_precondition("Restart reminders first"))?;
                let Some(row) = row else {
                    return Err(failed_precondition("Restart reminders first"));
                };
                let steps: Vec<i32> =
                    PaymentReminderStepRow::list_by_sequence_ids(conn, tenant_id, &[sequence_id])
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into_iter()
                        .map(|s| s.offset_days)
                        .collect();
                if !steps.contains(&offset_days) {
                    return Err(invalid(format!(
                        "Day {offset_days} is not a step of the invoice's reminder sequence"
                    )));
                }
                if invoice.consolidated_into_invoice_id.is_some() {
                    return Err(failed_precondition(
                        "A consolidated child invoice is never reminded on its own",
                    ));
                }
                if !is_payable(&invoice) {
                    return Err(failed_precondition("This invoice is not open for payment"));
                }
                let customer = CustomerRow::find_by_id(conn, &invoice.customer_id, &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if customer.payment_reminders_opt_out {
                    return Err(failed_precondition(
                        "The customer opted out of payment reminders",
                    ));
                }
                let settings = self
                    .internal
                    .get_email_settings_tx(conn, tenant_id, invoice.invoicing_entity_id)
                    .await?;
                if !settings.is_enabled(EmailKind::PaymentReminder) {
                    return Err(failed_precondition(
                        "Payment reminder emails are switched off for this invoicing entity",
                    ));
                }
                // A double click: the first submission was recorded (activity) or already
                // picked up by the worker (claim).
                if manual_reminder_recorded(conn, tenant_id, invoice_id, request_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    || EmailSendClaimRow::find_by_dedupe_key(conn, dedupe_key)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .is_some_and(|c| c.tenant_id == tenant_id)
                {
                    return Ok(());
                }

                self.enqueue_customer_email_tx(
                    conn,
                    tenant_id,
                    EmailKind::PaymentReminder,
                    EmailContext::PaymentReminder {
                        invoice_id,
                        sequence_id,
                        offset_days,
                        generation: row.generation,
                        manual: true,
                    },
                    dedupe_key.clone(),
                )
                .await?;

                // Only an in-flight ladder moves its cursor; a stopped, completed or ended row
                // keeps its schedule untouched.
                let in_flight = row.state == ReminderStateEnum::Enrolled
                    && row.stopped_at.is_none()
                    && row.completed_at.is_none();
                let advanced = in_flight && row.last_offset_days.is_none_or(|c| offset_days > c);
                let mut next = None;
                if advanced {
                    if let (Some(due_at), Some(finalized_at)) =
                        (invoice.due_at, invoice.finalized_at)
                    {
                        let window = settings.send_window();
                        let anchor = anchor_date(due_at, finalized_at, &window);
                        next = steps
                            .iter()
                            .filter(|o| **o > offset_days)
                            .map(|o| fire_time(anchor, *o, &window))
                            .find(|fire| schedulable(*fire, finalized_at));
                    }
                    InvoiceReminderUpdates::advance(
                        conn,
                        invoice_id,
                        tenant_id,
                        offset_days,
                        next,
                        now,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                }
                self.internal
                    .audit_invoice_reminder(
                        conn,
                        tenant_id,
                        actor,
                        ActivityType::PaymentReminderStepExecuted,
                        &invoice,
                        serde_json::json!({
                            "sequence_id": sequence_id.as_base62(),
                            "offset_days": offset_days,
                            "generation": row.generation,
                            "manual": true,
                            "request_id": request_id,
                            "next_reminder_at": if advanced { next } else { row.next_reminder_at },
                        }),
                    )
                    .await
            }
            .scope_boxed()
        })
        .await
    }
}

fn failed_precondition(msg: &str) -> Report<StoreError> {
    Report::new(StoreError::FailedPrecondition(msg.to_string()))
}

/// The invoice (customer first, then invoice) and its reminder row, both `FOR UPDATE`.
async fn lock_invoice_and_reminder(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> StoreResult<(InvoiceRow, Option<InvoicePaymentReminderRow>)> {
    let InvoiceLockRow { lock, invoice } =
        InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let row = InvoicePaymentReminderRow::find_by_invoice_id_for_update(conn, &lock, invoice_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok((invoice, row))
}

fn active_row(row: Option<InvoicePaymentReminderRow>) -> StoreResult<InvoicePaymentReminderRow> {
    match row {
        Some(row) if row.state == ReminderStateEnum::Enrolled && row.stopped_at.is_none() => {
            Ok(row)
        }
        Some(row) if row.stopped_at.is_some() => Err(failed_precondition(
            "Reminders are stopped for this invoice; restart them first",
        )),
        _ => Err(failed_precondition(
            "This invoice is not enrolled in reminders",
        )),
    }
}

impl StoreInternal {
    async fn audit_invoice_reminder(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        actor: &Actor,
        activity_type: ActivityType,
        invoice: &InvoiceRow,
        metadata: serde_json::Value,
    ) -> StoreResult<()> {
        let mut activity = Activity::new(activity_type, EntityType::Invoice, invoice.id.as_uuid())
            .with_metadata(metadata);
        activity.agg_customer_id = Some(invoice.customer_id);
        activity.agg_subscription_id = invoice.subscription_id;
        self.record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn entry(subject: &str) -> EmailCopy {
        EmailCopy {
            subject: subject.to_string(),
            body: "Hi {{customer_name}}".to_string(),
        }
    }

    #[test]
    fn content_needs_every_enabled_locale_and_keeps_extra_entries() {
        let content = EmailCopyByLocale(BTreeMap::from([
            ("fr-FR".to_string(), entry("Facture")),
            ("it-IT".to_string(), entry("Fattura")),
            ("de-DE".to_string(), entry("  ")),
        ]));
        assert!(validate_content(&content, 7, &["fr-FR"]).is_ok());
        let err = validate_content(&content, 7, &["fr-FR", "de-DE", "es-ES"]).unwrap_err();
        assert_eq!(
            err.current_context().to_string(),
            "Invalid Argument: Step +7: copy missing for de-DE, es-ES"
        );
        let err = validate_content(&content, -3, &["en-US"]).unwrap_err();
        assert!(
            err.current_context()
                .to_string()
                .contains("Step -3: copy missing for en-US")
        );
        assert!(validate_content(&EmailCopyByLocale::default(), 0, &["fr-FR", "de-DE"]).is_ok());
    }
}
