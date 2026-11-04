-- What usage does once the credit it draws on runs out: a plan version term, which a wallet can
-- override per customer.
CREATE TYPE "CreditExhaustedRuleEnum" AS ENUM ('INVOICE', 'PAUSE', 'INVOICE_UP_TO');
CREATE TYPE "UsageAtZeroEnum" AS ENUM ('FOLLOW_PLAN', 'ALWAYS_PAUSE', 'NEVER_PAUSE');

-- `credit_exhausted_rule` is unset only on a draft whose editor has not chosen it yet: publishing
-- sets the default. `credit_exhausted_caps`: with INVOICE_UP_TO, the usage beyond credit invoiced
-- per billing period, by denomination (upper-case currency or credit unit code), in minor units.
ALTER TABLE plan_version
    ADD COLUMN credit_exhausted_rule "CreditExhaustedRuleEnum",
    ADD COLUMN credit_exhausted_caps JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD CONSTRAINT plan_version_credit_exhausted_caps_check
        CHECK (jsonb_typeof(credit_exhausted_caps) = 'object'
            AND (credit_exhausted_rule = 'INVOICE_UP_TO' OR credit_exhausted_caps = '{}'::jsonb));

-- The default: a version that grants credit or prices usage in a credit unit pauses usage at zero.
UPDATE plan_version pv
SET credit_exhausted_rule = CASE
    WHEN EXISTS (
        SELECT 1
        FROM price_component pc
        JOIN plan_component_price pcp ON pcp.plan_component_id = pc.id
        JOIN price p ON p.id = pcp.price_id
        WHERE pc.plan_version_id = pv.id
          AND (p.credit_unit_id IS NOT NULL OR p.pricing ->> 'type' = 'Credit'))
    THEN 'PAUSE'::"CreditExhaustedRuleEnum"
    ELSE 'INVOICE'::"CreditExhaustedRuleEnum"
END;

-- A zero rate leaves credits beyond the balance unbilled, for a plan that pauses usage at zero.
ALTER TABLE price
    DROP CONSTRAINT price_credit_unit_rate_positive_check,
    ADD CONSTRAINT price_credit_unit_rate_check CHECK (credit_unit_rate >= 0);

ALTER TABLE customer_credit_account
    ADD COLUMN usage_at_zero "UsageAtZeroEnum" NOT NULL DEFAULT 'FOLLOW_PLAN';

UPDATE customer_credit_account SET usage_at_zero = 'ALWAYS_PAUSE' WHERE block_when_exhausted;

ALTER TABLE customer_credit_account DROP COLUMN block_when_exhausted;
