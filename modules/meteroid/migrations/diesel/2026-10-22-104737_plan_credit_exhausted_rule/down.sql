ALTER TABLE customer_credit_account
    ADD COLUMN block_when_exhausted BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE customer_credit_account SET block_when_exhausted = TRUE WHERE usage_at_zero = 'ALWAYS_PAUSE';

ALTER TABLE customer_credit_account DROP COLUMN usage_at_zero;

ALTER TABLE price
    DROP CONSTRAINT price_credit_unit_rate_check,
    ADD CONSTRAINT price_credit_unit_rate_positive_check CHECK (credit_unit_rate > 0) NOT VALID;

ALTER TABLE plan_version
    DROP CONSTRAINT plan_version_credit_exhausted_caps_check,
    DROP COLUMN credit_exhausted_caps,
    DROP COLUMN credit_exhausted_rule;

DROP TYPE "UsageAtZeroEnum";
DROP TYPE "CreditExhaustedRuleEnum";
