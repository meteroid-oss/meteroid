ALTER TABLE "subscription"
    ADD COLUMN "mrr_cents" BIGINT NOT NULL DEFAULT 0;

ALTER TABLE "subscription"
    DROP COLUMN "effective_billing_period",
    DROP COLUMN "input_parameters"
;
