DROP TABLE email_kind_audience;
DROP TYPE "EmailKindAudienceModeEnum";

DROP INDEX idx_invoice_reminder_candidates;

ALTER TABLE invoice DROP COLUMN auto_charge_requested_at;

DROP INDEX idx_customer_payment_reminder_sequence;
ALTER TABLE customer
    DROP COLUMN payment_reminder_sequence_id,
    DROP COLUMN payment_reminders_opt_out;

DROP TABLE invoice_payment_reminder;
DROP TABLE payment_reminder_step;
DROP TABLE payment_reminder_sequence;

DROP TYPE "ReminderStopReasonEnum";
DROP TYPE "ReminderNotEnrolledReasonEnum";
DROP TYPE "ReminderStateEnum";
DROP TYPE "ReminderEnrolledViaEnum";
DROP TYPE "PaymentReminderStepActionEnum";

DROP TABLE email_send_claim;
DROP TYPE "EmailSendClaimStateEnum";

DROP INDEX idx_sent_email_dedupe;
DROP INDEX idx_sent_email_invoicing_entity_time;
ALTER TABLE sent_email
    DROP COLUMN resent_from,
    DROP COLUMN dedupe_key,
    DROP COLUMN context,
    DROP COLUMN invoicing_entity_id,
    DROP COLUMN locale,
    DROP COLUMN kind;

ALTER TABLE invoicing_entity
    DROP COLUMN if exists brand_color_light,
    DROP COLUMN  if exists  customer_extra_locales,
    DROP COLUMN if exists   customer_locale;

DROP TABLE if exists  invoicing_entity_email_settings;
