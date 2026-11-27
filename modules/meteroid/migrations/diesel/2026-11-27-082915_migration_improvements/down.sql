DROP INDEX migration_source_request_id_idx;
ALTER TABLE migration_source DROP COLUMN request_id;
ALTER TABLE migration_customer
    DROP COLUMN payment_method_checked_at,
    DROP COLUMN payment_method_check_reason,
    DROP COLUMN payment_method_check;
ALTER TABLE migration_subscription DROP COLUMN resume_at_source_by;

ALTER TABLE migration_source DROP COLUMN usage_from;

-- Rows using what this migration added: SUSPEND is the stricter breach action, and the earlier
-- schema has no Paddle or Chargebee source (their imports go with them).
UPDATE migration_source SET breach_action = 'SUSPEND' WHERE breach_action IN ('WARN', 'IGNORE');
DELETE FROM migration_source WHERE kind IN ('PADDLE', 'CHARGEBEE');

ALTER TABLE migration_source DROP CONSTRAINT migration_source_breach_action_check;
ALTER TABLE migration_source ADD CONSTRAINT migration_source_breach_action_check
    CHECK (breach_action IN ('SUSPEND', 'BLOCK'));

ALTER TABLE migration_source DROP CONSTRAINT migration_source_kind_check;
ALTER TABLE migration_source ADD CONSTRAINT migration_source_kind_check
    CHECK (kind IN ('LEMON_SQUEEZY', 'STRIPE', 'CSV'));
