//! `InvoiceReminderState`: the per-invoice read model behind `GetInvoiceReminders`, and the
//! resolution preview behind `PreviewInvoiceReminderResolution`.

use crate::StoreResult;
use crate::domain::customer_emails::EmailKind;
use crate::domain::payment_reminders::{
    InvoicePaymentReminder, InvoiceReminderState, InvoiceReminderStepStatus,
    InvoiceReminderStepView, NotEligibleReason, PaymentReminderSequence, ReminderState, SendWindow,
    SequenceRef,
};
use crate::errors::StoreError;
use crate::services::payment_reminders::audience::{compile, evaluate_audience};
use crate::services::payment_reminders::schedule::{anchor_date, fire_time, schedulable};
use crate::services::payment_reminders::sweep::{is_payable, is_reminder_eligible, offsets};
use crate::store::{PgConn, StoreInternal};
use chrono::NaiveDateTime;
use common_domain::ids::{EntityActivityId, InvoiceId, PaymentReminderSequenceId, TenantId};
use diesel_models::customers::CustomerRow;
use diesel_models::enums::TenantStatusEnum;
use diesel_models::invoices::InvoiceRow;
use diesel_models::payment_reminders::{
    InvoicePaymentReminderRow, PaymentReminderSequenceRow, PaymentReminderStepRow,
};
use diesel_models::query::payment_reminders::list_sent_reminders;
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use std::collections::HashMap;

/// Everything `not_eligible_reason` looks at, in the order it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EligibilityFacts {
    pub payable: bool,
    pub has_due_and_finalized: bool,
    pub has_pdf: bool,
    pub consolidated_child: bool,
    pub auto_charged: bool,
    pub opted_out: bool,
    pub stopped: bool,
    pub tenant_active: bool,
    pub emails_disabled: bool,
    pub kind_enabled: bool,
    pub has_enabled_sequence: bool,
    pub ladder_stale: bool,
}

/// The first applicable reason in enum order, so the badge text is stable (§4.2).
pub fn not_eligible_reason(f: &EligibilityFacts) -> Option<NotEligibleReason> {
    [
        (!f.payable, NotEligibleReason::NotPayable),
        (!f.has_due_and_finalized, NotEligibleReason::NoDueDate),
        (!f.has_pdf, NotEligibleReason::NoPdf),
        (f.consolidated_child, NotEligibleReason::ConsolidatedChild),
        (f.auto_charged, NotEligibleReason::AutoCharged),
        (f.opted_out, NotEligibleReason::CustomerOptedOut),
        (f.stopped, NotEligibleReason::Stopped),
        (!f.tenant_active, NotEligibleReason::TenantInactive),
        (f.emails_disabled, NotEligibleReason::EmailsDisabled),
        (!f.kind_enabled, NotEligibleReason::KindDisabled),
        (
            !f.has_enabled_sequence,
            NotEligibleReason::NoEnabledSequence,
        ),
        (f.ladder_stale, NotEligibleReason::LadderStale),
    ]
    .into_iter()
    .find_map(|(applies, reason)| applies.then_some(reason))
}

pub fn sequence_ref(row: &PaymentReminderSequenceRow) -> SequenceRef {
    SequenceRef {
        id: row.id,
        name: row.name.clone(),
        enabled: row.enabled,
        archived: row.archived_at.is_some(),
    }
}

fn sequence_ref_of(sequence: &PaymentReminderSequence) -> SequenceRef {
    SequenceRef {
        id: sequence.id,
        name: sequence.name.clone(),
        enabled: sequence.enabled,
        archived: sequence.archived_at.is_some(),
    }
}

/// Timeline of a ladder. Enrolled or ended rows: SENT / SKIPPED_CATCH_UP up to the cursor,
/// then NOT_SCHEDULABLE / DUE / SCHEDULED. Non-enrolled: SCHEDULED when still ahead, else
/// NOT_SCHEDULABLE.
#[allow(clippy::too_many_arguments)]
pub fn step_views(
    steps: &[i32],
    anchor: chrono::NaiveDate,
    finalized_at: NaiveDateTime,
    window: &SendWindow,
    now: NaiveDateTime,
    cursor: Option<i32>,
    sent: &HashMap<i32, EntityActivityId>,
    enrolled: bool,
) -> Vec<InvoiceReminderStepView> {
    steps
        .iter()
        .map(|&offset_days| {
            let fire_at = fire_time(anchor, offset_days, window);
            let (status, sent_email_id) = if !enrolled {
                let status = if fire_at > now {
                    InvoiceReminderStepStatus::Scheduled
                } else {
                    InvoiceReminderStepStatus::NotSchedulable
                };
                (status, None)
            } else if cursor.is_some_and(|c| offset_days <= c) {
                match sent.get(&offset_days) {
                    Some(id) => (InvoiceReminderStepStatus::Sent, Some(*id)),
                    None => (InvoiceReminderStepStatus::SkippedCatchUp, None),
                }
            } else if !schedulable(fire_at, finalized_at) {
                (InvoiceReminderStepStatus::NotSchedulable, None)
            } else if fire_at <= now {
                (InvoiceReminderStepStatus::Due, None)
            } else {
                (InvoiceReminderStepStatus::Scheduled, None)
            };
            InvoiceReminderStepView {
                offset_days,
                fire_at,
                status,
                sent_email_id,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceResolutionRow {
    pub sequence: SequenceRef,
    pub is_default: bool,
    pub matched: bool,
    pub would_enrol: bool,
}

impl StoreInternal {
    pub(crate) async fn invoice_reminder_state_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<InvoiceReminderState> {
        let invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let customer = CustomerRow::find_by_id(conn, &invoice.customer_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let tenant = TenantRow::find_by_id(conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let settings = self
            .get_email_settings_tx(conn, tenant_id, invoice.invoicing_entity_id)
            .await?;
        let window = settings.send_window();
        let row: Option<InvoicePaymentReminder> =
            InvoicePaymentReminderRow::find_by_invoice_id(conn, tenant_id, invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .map(Into::into);
        let rules = self
            .load_entity_rules(conn, tenant_id, invoice.invoicing_entity_id)
            .await?;

        let frozen = match row.as_ref().and_then(|r| r.sequence_id) {
            Some(id) => PaymentReminderSequenceRow::find_by_id(conn, tenant_id, id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?,
            None => None,
        };
        let enrolled = row
            .as_ref()
            .is_some_and(|r| r.state != ReminderState::NotEnrolled && r.sequence_id.is_some());
        let (would_resolve_to, would_resolve_via) = if enrolled {
            (None, None)
        } else {
            let resolved = self
                .resolve_sequences(
                    conn,
                    &rules,
                    &[(invoice_id, customer.payment_reminder_sequence_id)],
                )
                .await?;
            match resolved.get(&invoice_id) {
                Some(r) => (Some(sequence_ref_of(r.sequence)), Some(r.via)),
                None => (None, None),
            }
        };

        let anchor = match (invoice.due_at, invoice.finalized_at) {
            (Some(due), Some(finalized)) => Some((anchor_date(due, finalized, &window), finalized)),
            _ => None,
        };
        let steps = match (anchor, enrolled) {
            (Some((anchor, finalized_at)), true) => {
                let sequence_id = frozen.as_ref().map(|s| s.id);
                let offsets: Vec<i32> = match sequence_id {
                    Some(id) => {
                        PaymentReminderStepRow::list_by_sequence_ids(conn, tenant_id, &[id])
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .into_iter()
                            .map(|s| s.offset_days)
                            .collect()
                    }
                    None => vec![],
                };
                let sent: HashMap<i32, EntityActivityId> =
                    list_sent_reminders(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into_iter()
                        .rev()
                        .map(|r| (r.offset_days, EntityActivityId::from(r.sent_email_id)))
                        .collect();
                step_views(
                    &offsets,
                    anchor,
                    finalized_at,
                    &window,
                    now,
                    row.as_ref().and_then(|r| r.last_offset_days),
                    &sent,
                    true,
                )
            }
            (Some((anchor, finalized_at)), false) => {
                let offsets = would_resolve_to
                    .as_ref()
                    .and_then(|r| rules.find(r.id))
                    .map(offsets)
                    .unwrap_or_default();
                step_views(
                    &offsets,
                    anchor,
                    finalized_at,
                    &window,
                    now,
                    None,
                    &HashMap::new(),
                    false,
                )
            }
            (None, _) => vec![],
        };

        let facts = EligibilityFacts {
            payable: is_payable(&invoice),
            has_due_and_finalized: anchor.is_some(),
            has_pdf: invoice.pdf_document_id.is_some(),
            consolidated_child: invoice.consolidated_into_invoice_id.is_some(),
            auto_charged: !is_reminder_eligible(&invoice),
            opted_out: customer.payment_reminders_opt_out,
            stopped: row.as_ref().is_some_and(|r| r.stopped_at.is_some()),
            tenant_active: tenant.status == TenantStatusEnum::Active,
            emails_disabled: tenant.email_delivery_mode
                == diesel_models::enums::EmailDeliveryModeEnum::Off,
            kind_enabled: settings.is_enabled(EmailKind::PaymentReminder),
            has_enabled_sequence: if enrolled {
                frozen
                    .as_ref()
                    .is_some_and(|s| s.enabled && s.archived_at.is_none())
            } else {
                would_resolve_to.is_some()
            },
            ladder_stale: row
                .as_ref()
                .is_some_and(|r| r.state == ReminderState::NotEnrolled),
        };

        Ok(InvoiceReminderState {
            sequence: frozen.as_ref().map(sequence_ref),
            enrolled_at: row.as_ref().and_then(|r| r.enrolled_at),
            enrolled_via: row.as_ref().and_then(|r| r.enrolled_via),
            last_offset_days: row.as_ref().and_then(|r| r.last_offset_days),
            next_reminder_at: row.as_ref().and_then(|r| r.next_reminder_at),
            paused_until: row.as_ref().and_then(|r| r.paused_until),
            stopped_at: row.as_ref().and_then(|r| r.stopped_at),
            stop_reason: row.as_ref().and_then(|r| r.stop_reason),
            completed_at: row.as_ref().and_then(|r| r.completed_at),
            ended_at: row.as_ref().and_then(|r| r.ended_at),
            generation: row.as_ref().map_or(0, |r| r.generation),
            steps,
            not_eligible_reason: not_eligible_reason(&facts),
            would_resolve_to,
            would_resolve_via,
            send_window: window,
        })
    }

    /// Every live sequence of the entity in position order: whether its audience matches the
    /// invoice and which one would win (override first, then enabled sequences only).
    pub(crate) async fn preview_invoice_reminder_resolution_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<SequenceResolutionRow>> {
        let invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let customer = CustomerRow::find_by_id(conn, &invoice.customer_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let live = PaymentReminderSequenceRow::list_by_entity(
            conn,
            tenant_id,
            invoice.invoicing_entity_id,
            false,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let rules = self
            .load_entity_rules(conn, tenant_id, invoice.invoicing_entity_id)
            .await?;
        let winner: Option<PaymentReminderSequenceId> = self
            .resolve_sequences(
                conn,
                &rules,
                &[(invoice_id, customer.payment_reminder_sequence_id)],
            )
            .await?
            .get(&invoice_id)
            .map(|r| r.sequence.id);

        let mut rows = Vec::with_capacity(live.len());
        for sequence in &live {
            let matched = match &sequence.audience {
                None => true,
                Some(audience) => {
                    let audience = serde_json::from_value(audience.clone()).map_err(|e| {
                        StoreError::SerdeError("Failed to deserialize sequence audience".into(), e)
                    })?;
                    let compiled = compile(&audience)?;
                    !evaluate_audience(conn, tenant_id, &[invoice_id], &compiled)
                        .await?
                        .is_empty()
                }
            };
            rows.push(SequenceResolutionRow {
                sequence: sequence_ref(sequence),
                is_default: sequence.audience.is_none(),
                matched,
                would_enrol: winner == Some(sequence.id),
            });
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eligible() -> EligibilityFacts {
        EligibilityFacts {
            payable: true,
            has_due_and_finalized: true,
            has_pdf: true,
            consolidated_child: false,
            auto_charged: false,
            opted_out: false,
            stopped: false,
            tenant_active: true,
            emails_disabled: false,
            kind_enabled: true,
            has_enabled_sequence: true,
            ladder_stale: false,
        }
    }

    #[test]
    fn eligible_when_nothing_applies() {
        assert_eq!(not_eligible_reason(&eligible()), None);
    }

    #[test]
    fn first_reason_in_enum_order_wins() {
        let facts = EligibilityFacts {
            opted_out: true,
            emails_disabled: true,
            ladder_stale: true,
            ..eligible()
        };
        assert_eq!(
            not_eligible_reason(&facts),
            Some(NotEligibleReason::CustomerOptedOut)
        );
        let facts = EligibilityFacts {
            payable: false,
            ..facts
        };
        assert_eq!(
            not_eligible_reason(&facts),
            Some(NotEligibleReason::NotPayable)
        );
        let facts = EligibilityFacts {
            ladder_stale: true,
            ..eligible()
        };
        assert_eq!(
            not_eligible_reason(&facts),
            Some(NotEligibleReason::LadderStale)
        );
    }
}
