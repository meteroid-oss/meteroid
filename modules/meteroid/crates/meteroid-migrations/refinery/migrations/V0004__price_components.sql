-- Modify "plan_version" table
ALTER TABLE
    "plan_version"
ADD
    COLUMN "tenant_id" uuid NOT NULL,
    -- null == subscription anniversary
ADD
    COLUMN "period_start_day" smallint NULL,
ADD
    COLUMN "net_terms" integer NOT NULL,
ADD
    COLUMN "currency" text NOT NULL,
    -- Number of billing cycles before ending the subscription. Null means forever.
ADD
    COLUMN "billing_cycles" integer NULL,
    -- These are the billing frequencies options for linked subscriptions, not the actual billing periods, that can depend on components.
ADD
    COLUMN "billing_frequencies" "BillingFrequencyEnum" [] NOT NULL;

-- Create "price_component" table
CREATE TABLE "price_component" (
    "id" uuid NOT NULL,
    "name" text NOT NULL,
    "fee" jsonb NOT NULL,
    "plan_version_id" uuid NOT NULL,
    "product_item_id" uuid NULL,
    PRIMARY KEY ("id"),
    CONSTRAINT "price_component_plan_version_id_fkey" FOREIGN KEY ("plan_version_id") REFERENCES "plan_version" ("id") ON UPDATE CASCADE ON DELETE RESTRICT,
    CONSTRAINT "price_component_product_item_id_fkey" FOREIGN KEY ("product_item_id") REFERENCES "product" ("id") ON UPDATE CASCADE ON DELETE RESTRICT
);

-- Modify "price_ramp" table
ALTER TABLE
    "price_ramp" DROP COLUMN "price_point_id",
    DROP COLUMN "idx",
ADD
    COLUMN "plan_version_id" uuid NOT NULL,
ADD
    COLUMN "billing_frequency" "BillingFrequencyEnum" NOT NULL,
ADD
    COLUMN "fractional_index" text NOT NULL,
ADD
    CONSTRAINT "price_ramp_plan_version_id_fkey" FOREIGN KEY ("plan_version_id") REFERENCES "plan_version" ("id") ON UPDATE CASCADE ON DELETE RESTRICT;

-- Modify "subscription" table
ALTER TABLE
    "subscription" DROP COLUMN "price_point_id",
ADD
    COLUMN "plan_version_id" uuid NOT NULL,
ADD
    CONSTRAINT "subscription_plan_version_id_fkey" FOREIGN KEY ("plan_version_id") REFERENCES "plan_version" ("id") ON UPDATE CASCADE ON DELETE RESTRICT;

-- Drop "billable_metric_to_product" table
DROP TABLE "billable_metric_to_product";

-- Drop "plan_to_product" table
DROP TABLE "plan_to_product";

-- Drop "product_charge" table
DROP TABLE "product_charge";

-- Drop "priced_product" table
DROP TABLE "priced_product";

-- Drop "price_point" table
DROP TABLE "price_point";

-- Drop enum type "BillingCycleEnum"
DROP TYPE "BillingCycleEnum";

-- Drop enum type "ServicePeriodStartOnEnum"
DROP TYPE "ServicePeriodStartOnEnum";

-- Modify "subscription" table
ALTER TABLE
    "subscription"
ADD
    COLUMN "parameters_todo" jsonb NULL,
ADD
    COLUMN "effective_billing_frequency" "BillingFrequencyEnum" NOT NULL;

-- Modify "plan_version" table
ALTER TABLE
    "plan_version"
ADD
    COLUMN "created_at" timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
ADD
    COLUMN "created_by" uuid NOT NULL;

-- Modify "price_component" table
ALTER TABLE
    "price_component" DROP CONSTRAINT "price_component_plan_version_id_fkey",
    DROP CONSTRAINT "price_component_product_item_id_fkey",
ADD
    CONSTRAINT "price_component_plan_version_id_fkey" FOREIGN KEY ("plan_version_id") REFERENCES "plan_version" ("id") ON UPDATE CASCADE ON DELETE CASCADE,
ADD
    CONSTRAINT "price_component_product_item_id_fkey" FOREIGN KEY ("product_item_id") REFERENCES "product" ("id") ON UPDATE CASCADE ON DELETE
SET
    NULL;

-- Modify "price_ramp" table
ALTER TABLE
    "price_ramp" DROP CONSTRAINT "price_ramp_plan_version_id_fkey",
ADD
    CONSTRAINT "price_ramp_plan_version_id_fkey" FOREIGN KEY ("plan_version_id") REFERENCES "plan_version" ("id") ON UPDATE CASCADE ON DELETE CASCADE;

-- Modify "price_ramp" table
ALTER TABLE "price_ramp" DROP COLUMN "name", DROP COLUMN "discount", DROP COLUMN "minimum", DROP COLUMN "free_credit", ADD COLUMN "ramp_adjustments" jsonb NULL;

-- Modify "tenant" table
ALTER TABLE "tenant" ADD COLUMN "currency" text NOT NULL;


CREATE VIEW current_billing_period AS
WITH billing_periods AS (
    SELECT
        id AS subscription_id,
        billing_start_date,
        effective_billing_frequency,
        CASE
            WHEN effective_billing_frequency = 'MONTHLY' THEN
                        (EXTRACT(YEAR FROM AGE(now(), billing_start_date)) * 12 + EXTRACT(MONTH FROM AGE(now(), billing_start_date)))::INTEGER
            WHEN effective_billing_frequency = 'ANNUAL' THEN
                EXTRACT(YEAR FROM AGE(now(), billing_start_date))::INTEGER
            END AS current_period_idx
    FROM subscription
)
SELECT
    subscription_id,
    billing_start_date + INTERVAL '1' MONTH * current_period_idx AS current_period_start_date,
    CASE
        WHEN effective_billing_frequency = 'MONTHLY' THEN
                    billing_start_date + INTERVAL '1' MONTH * (current_period_idx + 1) - INTERVAL '1 day'
        WHEN effective_billing_frequency = 'ANNUAL' THEN
                    billing_start_date + INTERVAL '1' YEAR * (current_period_idx + 1) - INTERVAL '1 day'
        END AS current_period_end_date,
    current_period_idx
FROM
    billing_periods;