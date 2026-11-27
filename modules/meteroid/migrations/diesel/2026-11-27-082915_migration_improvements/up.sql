ALTER TABLE migration_source DROP CONSTRAINT migration_source_kind_check;
ALTER TABLE migration_source ADD CONSTRAINT migration_source_kind_check
    CHECK (kind IN ('LEMON_SQUEEZY', 'STRIPE', 'CSV', 'PADDLE', 'CHARGEBEE'));

-- WARN lets a period over tolerance through and records it; IGNORE lets it through.
ALTER TABLE migration_source DROP CONSTRAINT migration_source_breach_action_check;
ALTER TABLE migration_source ADD CONSTRAINT migration_source_breach_action_check
    CHECK (breach_action IN ('SUSPEND', 'BLOCK', 'WARN', 'IGNORE'));

-- Where shadow billing reads usage: what the source counted, or Meteroid's metering (dual-send).
ALTER TABLE migration_source
    ADD COLUMN usage_from TEXT NOT NULL DEFAULT 'SOURCE' CHECK (usage_from IN ('SOURCE', 'METERING'));

-- A rolled-back subscription its source didn't resume: the operator resumes it there by then.
ALTER TABLE migration_subscription ADD COLUMN resume_at_source_by TIMESTAMPTZ;

-- The imported card, looked up through Meteroid's own Stripe connector.
ALTER TABLE migration_customer
    ADD COLUMN payment_method_check TEXT
        CHECK (payment_method_check IN ('VERIFIED', 'NOT_FOUND', 'OTHER_CUSTOMER', 'DETACHED', 'UNVERIFIABLE')),
    ADD COLUMN payment_method_check_reason TEXT,
    ADD COLUMN payment_method_checked_at TIMESTAMPTZ;

-- A retried CreateSource with the same request id returns the source it created.
ALTER TABLE migration_source ADD COLUMN request_id TEXT;
CREATE UNIQUE INDEX migration_source_request_id_idx ON migration_source (tenant_id, request_id)
    WHERE request_id IS NOT NULL;
