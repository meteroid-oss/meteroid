pub mod sequences {
    use crate::paymentreminders::error::PaymentRemindersApiError;
    use meteroid_grpc::meteroid::api::customeremails::v1 as emails;
    use meteroid_grpc::meteroid::api::paymentreminders::v1 as server;
    use meteroid_store::domain::payment_reminders as domain;
    use meteroid_store::repositories::payment_reminders::{
        AudiencePreview, CustomerReminderPreferenceView, SequenceResolutionRow,
        SequenceWithInFlight,
    };
    use meteroid_store::services::payment_reminders::audience::parse_audience;

    type Result<T> = std::result::Result<T, PaymentRemindersApiError>;

    fn timestamp(t: chrono::NaiveDateTime) -> String {
        t.and_utc().to_rfc3339()
    }

    pub fn copy_by_locale_from_proto(
        content: Option<emails::EmailCopyByLocale>,
    ) -> domain::EmailCopyByLocale {
        domain::EmailCopyByLocale(
            content
                .map(|c| c.by_locale)
                .unwrap_or_default()
                .into_iter()
                .map(|(locale, copy)| {
                    (
                        locale,
                        domain::EmailCopy {
                            subject: copy.subject,
                            body: copy.body,
                        },
                    )
                })
                .collect(),
        )
    }

    fn copy_by_locale_to_proto(content: domain::EmailCopyByLocale) -> emails::EmailCopyByLocale {
        emails::EmailCopyByLocale {
            by_locale: content
                .0
                .into_iter()
                .map(|(locale, copy)| {
                    (
                        locale,
                        emails::EmailCopy {
                            subject: copy.subject,
                            body: copy.body,
                        },
                    )
                })
                .collect(),
        }
    }

    pub fn steps_from_proto(
        steps: Vec<server::PaymentReminderStepInput>,
    ) -> Vec<domain::PaymentReminderStepInput> {
        steps
            .into_iter()
            .map(|s| domain::PaymentReminderStepInput {
                offset_days: s.offset_days,
                content: copy_by_locale_from_proto(s.content),
            })
            .collect()
    }

    pub fn audience_from_proto(json: &str) -> Result<domain::Audience> {
        parse_audience(json).map_err(Into::into)
    }

    pub fn domain_to_proto(
        listed: SequenceWithInFlight,
    ) -> Result<server::PaymentReminderSequence> {
        let s = listed.sequence;
        let audience = s
            .audience
            .as_ref()
            .map(domain::Audience::to_api_json)
            .transpose()
            .map_err(|e| {
                PaymentRemindersApiError::StoreError(
                    "Failed to serialize audience".to_string(),
                    Box::new(e),
                )
            })?;
        Ok(server::PaymentReminderSequence {
            id: s.id.as_proto(),
            invoicing_entity_id: s.invoicing_entity_id.as_proto(),
            name: s.name,
            position: s.position,
            enabled: s.enabled,
            is_default: audience.is_none(),
            audience,
            steps: s
                .steps
                .into_iter()
                .map(|step| server::PaymentReminderStep {
                    id: step.id.as_proto(),
                    offset_days: step.offset_days,
                    action: server::PaymentReminderStepAction::Notify as i32,
                    content: Some(copy_by_locale_to_proto(step.content)),
                })
                .collect(),
            created_at: timestamp(s.created_at),
            updated_at: s.updated_at.map(timestamp),
            archived_at: s.archived_at.map(timestamp),
            in_flight_invoices: listed.in_flight_invoices.min(i32::MAX as i64) as i32,
        })
    }

    pub fn sequence_ref_to_proto(r: domain::SequenceRef) -> server::SequenceRef {
        server::SequenceRef {
            id: r.id.as_proto(),
            name: r.name,
            enabled: r.enabled,
            archived: r.archived,
        }
    }

    pub fn preview_to_proto(p: AudiencePreview) -> server::AudiencePreview {
        server::AudiencePreview {
            invoice_count: p.invoice_count as i32,
            customer_count: p.customer_count as i32,
            claimed_by_earlier: p.claimed_by_earlier as i32,
            sample: p
                .sample
                .into_iter()
                .map(|r| server::AudiencePreviewRow {
                    invoice_id: r.invoice_id.as_proto(),
                    invoice_number: r.invoice_number,
                    customer_id: r.customer_id.as_proto(),
                    customer_name: r.customer_name,
                    amount_due: r.amount_due,
                    currency: r.currency,
                })
                .collect(),
        }
    }

    fn enrolled_via_to_proto(via: Option<domain::ReminderEnrolledVia>) -> i32 {
        match via {
            None => server::ReminderEnrolledVia::Unspecified,
            Some(domain::ReminderEnrolledVia::CustomerOverride) => {
                server::ReminderEnrolledVia::CustomerOverride
            }
            Some(domain::ReminderEnrolledVia::Audience) => server::ReminderEnrolledVia::Audience,
            Some(domain::ReminderEnrolledVia::Default) => server::ReminderEnrolledVia::Default,
        }
        .into()
    }

    fn stop_reason_to_proto(reason: Option<domain::ReminderStopReason>) -> i32 {
        match reason {
            None => server::ReminderStopReason::Unspecified,
            Some(domain::ReminderStopReason::Operator) => server::ReminderStopReason::Operator,
            Some(domain::ReminderStopReason::CustomerOptOut) => {
                server::ReminderStopReason::CustomerOptOut
            }
            Some(domain::ReminderStopReason::SequenceArchived) => {
                server::ReminderStopReason::SequenceArchived
            }
            Some(domain::ReminderStopReason::PaymentDisputed) => {
                server::ReminderStopReason::PaymentDisputed
            }
        }
        .into()
    }

    fn step_status_to_proto(status: domain::InvoiceReminderStepStatus) -> i32 {
        match status {
            domain::InvoiceReminderStepStatus::Sent => server::InvoiceReminderStepStatus::Sent,
            domain::InvoiceReminderStepStatus::SkippedCatchUp => {
                server::InvoiceReminderStepStatus::SkippedCatchUp
            }
            domain::InvoiceReminderStepStatus::Scheduled => {
                server::InvoiceReminderStepStatus::Scheduled
            }
            domain::InvoiceReminderStepStatus::Due => server::InvoiceReminderStepStatus::Due,
            domain::InvoiceReminderStepStatus::NotSchedulable => {
                server::InvoiceReminderStepStatus::NotSchedulable
            }
        }
        .into()
    }

    fn not_eligible_reason_to_proto(reason: Option<domain::NotEligibleReason>) -> i32 {
        use domain::NotEligibleReason as D;
        use server::NotEligibleReason as P;
        match reason {
            None => P::Unspecified,
            Some(D::NotPayable) => P::NotPayable,
            Some(D::NoDueDate) => P::NoDueDate,
            Some(D::NoPdf) => P::NoPdf,
            Some(D::ConsolidatedChild) => P::ConsolidatedChild,
            Some(D::AutoCharged) => P::AutoCharged,
            Some(D::CustomerOptedOut) => P::CustomerOptedOut,
            Some(D::Stopped) => P::Stopped,
            Some(D::TenantInactive) => P::TenantInactive,
            Some(D::EmailsDisabled) => P::EmailsDisabled,
            Some(D::KindDisabled) => P::KindDisabled,
            Some(D::NoEnabledSequence) => P::NoEnabledSequence,
            Some(D::LadderStale) => P::LadderStale,
        }
        .into()
    }

    pub fn invoice_state_to_proto(s: domain::InvoiceReminderState) -> server::InvoiceReminderState {
        let eligible = s.eligible();
        server::InvoiceReminderState {
            sequence: s.sequence.map(sequence_ref_to_proto),
            enrolled_at: s.enrolled_at.map(timestamp),
            enrolled_via: enrolled_via_to_proto(s.enrolled_via),
            last_offset_days: s.last_offset_days,
            next_reminder_at: s.next_reminder_at.map(timestamp),
            paused_until: s.paused_until.map(timestamp),
            stopped_at: s.stopped_at.map(timestamp),
            stop_reason: stop_reason_to_proto(s.stop_reason),
            completed_at: s.completed_at.map(timestamp),
            ended_at: s.ended_at.map(timestamp),
            generation: s.generation,
            steps: s
                .steps
                .into_iter()
                .map(|step| server::InvoiceReminderStepView {
                    offset_days: step.offset_days,
                    fire_at: timestamp(step.fire_at),
                    status: step_status_to_proto(step.status),
                    sent_email_id: step.sent_email_id.map(|id| id.as_proto()),
                })
                .collect(),
            eligible,
            not_eligible_reason: not_eligible_reason_to_proto(s.not_eligible_reason),
            would_resolve_to: s.would_resolve_to.map(sequence_ref_to_proto),
            would_resolve_via: enrolled_via_to_proto(s.would_resolve_via),
            send_window: s.send_window.to_string(),
        }
    }

    pub fn resolution_row_to_proto(r: SequenceResolutionRow) -> server::SequenceResolutionRow {
        server::SequenceResolutionRow {
            sequence: Some(sequence_ref_to_proto(r.sequence)),
            is_default: r.is_default,
            matched: r.matched,
            would_enrol: r.would_enrol,
        }
    }

    /// RFC 3339 (any offset) to naive UTC.
    pub fn parse_timestamp(field: &str, value: &str) -> Result<chrono::NaiveDateTime> {
        chrono::DateTime::parse_from_rfc3339(value.trim())
            .map(|t| t.naive_utc())
            .map_err(|_| {
                PaymentRemindersApiError::InvalidArgument(format!(
                    "{field} must be an RFC 3339 timestamp"
                ))
            })
    }

    pub fn preference_to_proto(
        view: CustomerReminderPreferenceView,
    ) -> server::CustomerReminderPreference {
        server::CustomerReminderPreference {
            opt_out: view.preference.opt_out,
            sequence_id: view.preference.sequence_id.map(|id| id.as_proto()),
            available_sequences: view
                .available_sequences
                .into_iter()
                .map(sequence_ref_to_proto)
                .collect(),
        }
    }
}
