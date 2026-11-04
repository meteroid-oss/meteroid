-- One row per invoicing entity, created lazily. Absent row = defaults:
-- sender = legal_name, no reply-to, no bcc, tz UTC, 09:00, disabled_kinds = the platform-disabled set below.
-- The column default IS that set so backfill, lazy upserts and EmailSettings::defaults agree.
CREATE TABLE invoicing_entity_email_settings (
    invoicing_entity_id UUID PRIMARY KEY REFERENCES invoicing_entity(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    sender_name TEXT,
    reply_to TEXT,
    bcc TEXT[] NOT NULL DEFAULT '{}' CHECK (cardinality(bcc) <= 5),
    -- EmailKind snake_case values switched off for this entity (validated against the Rust enum at save)
    disabled_kinds TEXT[] NOT NULL DEFAULT '{payment_reminder}',
    -- IANA zone for the reminder send hour; NULL = UTC
    timezone TEXT,
    reminder_send_hour SMALLINT NOT NULL DEFAULT 9 CHECK (reminder_send_hour BETWEEN 0 AND 23),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP
);
CREATE INDEX idx_invoicing_entity_email_settings_tenant ON invoicing_entity_email_settings (tenant_id);

-- Customer-facing language (emails, portal), decoupled from the invoice language. NULL = same as the invoice.
-- Extra locales are the ones a customer's preference may pick; both validated against the template locales at save.
-- brand_color_light: brand colour for white backgrounds (emails). NULL = derived from brand_color.
ALTER TABLE invoicing_entity
    ADD COLUMN customer_locale TEXT,
    ADD COLUMN customer_extra_locales TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN brand_color_light TEXT;

-- Every existing entity gets its settings row: all kinds on except payment reminders, which need
-- a deliberate sequence. Tenants that want no customer email keep the workspace-wide switch.
INSERT INTO invoicing_entity_email_settings (invoicing_entity_id, tenant_id, disabled_kinds)
SELECT id, tenant_id, '{payment_reminder}'
FROM invoicing_entity
ON CONFLICT (invoicing_entity_id) DO NOTHING;

ALTER TABLE sent_email
    ADD COLUMN kind TEXT,
    ADD COLUMN locale TEXT,
    ADD COLUMN invoicing_entity_id UUID REFERENCES invoicing_entity(id) ON DELETE SET NULL,
    -- EmailContext JSON: what is needed to re-render (invoice id, offset_days, payment_transaction_id, ...)
    ADD COLUMN context JSONB,
    ADD COLUMN dedupe_key TEXT,
    ADD COLUMN resent_from UUID REFERENCES sent_email(id) ON DELETE SET NULL;
UPDATE sent_email s SET kind = a.metadata ->> 'kind' FROM entity_activity a WHERE a.id = s.id;
-- Only invoice emails have been recorded so far; the per-entity listing filters on this column.
UPDATE sent_email s SET invoicing_entity_id = i.invoicing_entity_id
FROM entity_activity a JOIN invoice i ON i.id = a.entity_id
WHERE a.id = s.id AND a.entity_type = 'invoice';
CREATE INDEX idx_sent_email_invoicing_entity_time ON sent_email (tenant_id, invoicing_entity_id, sent_at DESC);
CREATE UNIQUE INDEX idx_sent_email_dedupe ON sent_email (dedupe_key) WHERE dedupe_key IS NOT NULL;

-- Claim-before-send. One row per dedupe_key, upserted before SMTP.
-- SENDING = in flight; SENT = done; FAILED = released (skip/defer/transport error), re-claimable; UNKNOWN = stale, terminal.
CREATE TYPE "EmailSendClaimStateEnum" AS ENUM ('SENDING', 'SENT', 'FAILED', 'UNKNOWN');
CREATE TABLE email_send_claim (
    dedupe_key TEXT PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    state "EmailSendClaimStateEnum" NOT NULL DEFAULT 'SENDING',
    claimed_at TIMESTAMP NOT NULL,
    finished_at TIMESTAMP,
    sent_email_id UUID REFERENCES sent_email(id) ON DELETE SET NULL
);
CREATE INDEX idx_email_send_claim_tenant ON email_send_claim (tenant_id);

CREATE TYPE "PaymentReminderStepActionEnum" AS ENUM ('NOTIFY');          -- RETRY, RETRY_AND_NOTIFY reserved for the dunning slice
CREATE TYPE "ReminderEnrolledViaEnum"    AS ENUM ('CUSTOMER_OVERRIDE', 'AUDIENCE', 'DEFAULT');
CREATE TYPE "ReminderStateEnum"          AS ENUM ('NOT_ENROLLED', 'ENROLLED', 'ENDED');
CREATE TYPE "ReminderNotEnrolledReasonEnum" AS ENUM ('LADDER_STALE');                 -- the only stamped negative resolution
CREATE TYPE "ReminderStopReasonEnum"     AS ENUM ('OPERATOR', 'CUSTOMER_OPT_OUT', 'SEQUENCE_ARCHIVED');

CREATE TABLE payment_reminder_sequence (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    invoicing_entity_id UUID NOT NULL REFERENCES invoicing_entity(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    -- evaluation order; the default (audience IS NULL) always has the highest live position
    position INT NOT NULL,
    -- disabled = skipped at resolution; invoices already frozen on it are held, not moved
    enabled BOOLEAN NOT NULL DEFAULT false,
    -- Audience JSON; NULL = the entity's default sequence, matches every invoice
    audience JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP,
    archived_at TIMESTAMP
);
CREATE UNIQUE INDEX idx_payment_reminder_sequence_position ON payment_reminder_sequence (invoicing_entity_id, position) WHERE archived_at IS NULL;
CREATE UNIQUE INDEX idx_payment_reminder_sequence_default  ON payment_reminder_sequence (invoicing_entity_id) WHERE audience IS NULL AND archived_at IS NULL;
CREATE INDEX idx_payment_reminder_sequence_tenant ON payment_reminder_sequence (tenant_id);

CREATE TABLE payment_reminder_step (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    sequence_id UUID NOT NULL REFERENCES payment_reminder_sequence(id) ON DELETE CASCADE,
    offset_days INT NOT NULL CHECK (offset_days BETWEEN -30 AND 365),
    action "PaymentReminderStepActionEnum" NOT NULL DEFAULT 'NOTIFY',
    -- EmailCopyByLocale: {"en-US":{"subject":"...","body":"..."}, ...}, keys ⊆ SUPPORTED
    content JSONB NOT NULL DEFAULT '{}'::jsonb,
    UNIQUE (sequence_id, offset_days)
);

-- 1:1 engine state per invoice. A row exists once the sweeper has enrolled the invoice or stamped it LADDER_STALE;
-- an invoice with no enabled winner has no row and is re-evaluated by the next sweep.
CREATE TABLE invoice_payment_reminder (
    invoice_id UUID PRIMARY KEY REFERENCES invoice(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    state "ReminderStateEnum" NOT NULL,
    not_enrolled_reason "ReminderNotEnrolledReasonEnum",
    resolved_at TIMESTAMP NOT NULL,
    -- bumped on enrol and restart; part of the dedupe key
    generation INT NOT NULL DEFAULT 0,
    sequence_id UUID REFERENCES payment_reminder_sequence(id),
    enrolled_via "ReminderEnrolledViaEnum",
    enrolled_at TIMESTAMP,
    last_offset_days INT,                 -- cursor = last EXECUTED offset, never an index
    next_reminder_at TIMESTAMP,           -- the only claim key; NULL = nothing scheduled
    paused_until TIMESTAMP,
    stopped_at TIMESTAMP,
    stop_reason "ReminderStopReasonEnum",
    completed_at TIMESTAMP,
    ended_at TIMESTAMP,                   -- paid / void / uncollectible while enrolled
    CHECK (state <> 'NOT_ENROLLED' OR not_enrolled_reason IS NOT NULL)
);
CREATE INDEX idx_invoice_payment_reminder_due ON invoice_payment_reminder (next_reminder_at)
    WHERE next_reminder_at IS NOT NULL AND stopped_at IS NULL;
CREATE INDEX idx_invoice_payment_reminder_sequence ON invoice_payment_reminder (sequence_id) WHERE sequence_id IS NOT NULL;

ALTER TABLE customer
    ADD COLUMN payment_reminders_opt_out BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN payment_reminder_sequence_id UUID REFERENCES payment_reminder_sequence(id) ON DELETE SET NULL;
CREATE INDEX idx_customer_payment_reminder_sequence ON customer (payment_reminder_sequence_id) WHERE payment_reminder_sequence_id IS NOT NULL;

-- The collection decision taken at finalization. NULL = pay-link invoice.
ALTER TABLE invoice ADD COLUMN auto_charge_requested_at TIMESTAMP;
-- Analytics does not project this column, so the backfill must not re-stamp store_committed_at.
ALTER TABLE invoice DISABLE TRIGGER trg_invoice_store_committed_at;
UPDATE invoice i SET auto_charge_requested_at = pt.first_at
FROM (SELECT invoice_id, MIN(created_at) AS first_at FROM payment_transaction
      WHERE payment_type = 'PAYMENT' AND payment_method_id IS NOT NULL AND initiated_by_customer_id IS NULL
      GROUP BY invoice_id) pt
WHERE pt.invoice_id = i.id;
ALTER TABLE invoice ENABLE TRIGGER trg_invoice_store_committed_at;

-- Enrolment scan: per entity, keyset on (anchor, id), bounded by the entity's ladder reach.
CREATE INDEX idx_invoice_reminder_candidates
    ON invoice (invoicing_entity_id, (GREATEST(due_at, finalized_at)), id)
    WHERE status = 'FINALIZED' AND payment_status IN ('UNPAID','PARTIALLY_PAID','ERRORED')
      AND consolidated_into_invoice_id IS NULL AND due_at IS NOT NULL AND finalized_at IS NOT NULL;

-- Per-entity "Send to" rule of an email kind. Absent row = Everyone.
-- payment_reminder targets audiences through its sequences and never has a row here.
CREATE TYPE "EmailKindAudienceModeEnum" AS ENUM ('EXCLUDE', 'ONLY');
CREATE TABLE email_kind_audience (
    invoicing_entity_id UUID NOT NULL REFERENCES invoicing_entity(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind <> 'payment_reminder'),
    mode "EmailKindAudienceModeEnum" NOT NULL,
    -- Same JSON as payment_reminder_sequence.audience, validated at save
    audience JSONB NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (invoicing_entity_id, kind)
);
CREATE INDEX idx_email_kind_audience_tenant ON email_kind_audience (tenant_id);
