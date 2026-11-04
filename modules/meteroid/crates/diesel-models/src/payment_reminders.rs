use crate::enums::{
    PaymentReminderStepActionEnum, ReminderEnrolledViaEnum, ReminderNotEnrolledReasonEnum,
    ReminderStateEnum, ReminderStopReasonEnum,
};
use chrono::NaiveDateTime;
use common_domain::ids::{
    InvoiceId, InvoicingEntityId, PaymentReminderSequenceId, PaymentReminderStepId, TenantId,
};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::payment_reminder_sequence)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentReminderSequenceRow {
    pub id: PaymentReminderSequenceId,
    pub tenant_id: TenantId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub name: String,
    /// Evaluation order; the default (`audience IS NULL`) always has the highest live position.
    pub position: i32,
    pub enabled: bool,
    /// Audience JSON; NULL = the entity's default sequence.
    pub audience: Option<serde_json::Value>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::payment_reminder_sequence)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentReminderSequenceRowNew {
    pub id: PaymentReminderSequenceId,
    pub tenant_id: TenantId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub name: String,
    pub position: i32,
    pub enabled: bool,
    pub audience: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::payment_reminder_step)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentReminderStepRow {
    pub id: PaymentReminderStepId,
    pub tenant_id: TenantId,
    pub sequence_id: PaymentReminderSequenceId,
    pub offset_days: i32,
    pub action: PaymentReminderStepActionEnum,
    /// `EmailCopyByLocale` JSON keyed by template locale.
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::payment_reminder_step)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentReminderStepRowNew {
    pub id: PaymentReminderStepId,
    pub tenant_id: TenantId,
    pub sequence_id: PaymentReminderSequenceId,
    pub offset_days: i32,
    pub action: PaymentReminderStepActionEnum,
    pub content: serde_json::Value,
}

/// 1:1 engine state per invoice; exists once the sweeper enrolled the invoice or stamped it.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::invoice_payment_reminder)]
#[diesel(primary_key(invoice_id))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicePaymentReminderRow {
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub state: ReminderStateEnum,
    pub not_enrolled_reason: Option<ReminderNotEnrolledReasonEnum>,
    pub resolved_at: NaiveDateTime,
    /// Bumped on enrol and restart; part of the dedupe key.
    pub generation: i32,
    pub sequence_id: Option<PaymentReminderSequenceId>,
    pub enrolled_via: Option<ReminderEnrolledViaEnum>,
    pub enrolled_at: Option<NaiveDateTime>,
    /// Cursor = last executed offset, never an index.
    pub last_offset_days: Option<i32>,
    /// The only claim key; NULL = nothing scheduled.
    pub next_reminder_at: Option<NaiveDateTime>,
    pub paused_until: Option<NaiveDateTime>,
    pub stopped_at: Option<NaiveDateTime>,
    pub stop_reason: Option<ReminderStopReasonEnum>,
    pub completed_at: Option<NaiveDateTime>,
    pub ended_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::invoice_payment_reminder)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicePaymentReminderRowNew {
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub state: ReminderStateEnum,
    pub not_enrolled_reason: Option<ReminderNotEnrolledReasonEnum>,
    pub resolved_at: NaiveDateTime,
    pub generation: i32,
    pub sequence_id: Option<PaymentReminderSequenceId>,
    pub enrolled_via: Option<ReminderEnrolledViaEnum>,
    pub enrolled_at: Option<NaiveDateTime>,
    pub last_offset_days: Option<i32>,
    pub next_reminder_at: Option<NaiveDateTime>,
    pub completed_at: Option<NaiveDateTime>,
}
