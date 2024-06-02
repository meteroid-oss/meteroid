-- Create enum type "BillingPeriodEnum"
CREATE TYPE "BillingPeriodEnum" AS ENUM ('MONTHLY', 'QUARTERLY', 'ANNUAL');

-- Modify "subscription" table
ALTER TABLE
   "subscription" DROP COLUMN "plan_id",
   DROP COLUMN "parameters_todo",
   DROP COLUMN "effective_billing_frequency" CASCADE,
ADD
   COLUMN "created_at" timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
ADD
   COLUMN "created_by" uuid NOT NULL,
ADD
   COLUMN "input_parameters" jsonb NULL,
ADD
   COLUMN "effective_billing_period" "BillingPeriodEnum" NOT NULL,
ADD
   COLUMN "net_terms" integer NOT NULL,
ADD
   COLUMN "invoice_memo" text NULL,
ADD
   COLUMN "invoice_threshold" numeric NULL;

-- Modify "plan_version" table
ALTER TABLE
   "plan_version" DROP COLUMN "billing_frequencies",
ADD
   COLUMN "billing_periods" "BillingPeriodEnum" [] NOT NULL;

-- Create "schedule" table
CREATE TABLE "schedule" (
   "id" uuid NOT NULL,
   "billing_period" "BillingPeriodEnum" NOT NULL,
   "plan_version_id" uuid NOT NULL,
   "ramps" jsonb NOT NULL,
   PRIMARY KEY ("id"),
   CONSTRAINT "schedule_plan_version_id_fkey" FOREIGN KEY ("plan_version_id") REFERENCES "plan_version" ("id") ON UPDATE CASCADE ON DELETE CASCADE
);

-- Drop "price_ramp" table
DROP TABLE "price_ramp";

-- Modify "tenant_invite" table
ALTER TABLE
   "tenant_invite" DROP CONSTRAINT "tenant_invite_tenant_id_fkey",
ADD
   CONSTRAINT "tenant_invite_tenant_id_fkey" FOREIGN KEY ("tenant_id") REFERENCES "tenant" ("id") ON UPDATE CASCADE ON DELETE CASCADE;

-- Modify "tenant_invite_link" table
ALTER TABLE
   "tenant_invite_link" DROP CONSTRAINT "tenant_invite_link_tenant_id_fkey",
ADD
   CONSTRAINT "tenant_invite_link_tenant_id_fkey" FOREIGN KEY ("tenant_id") REFERENCES "tenant" ("id") ON UPDATE CASCADE ON DELETE CASCADE;

-- Modify "current_billing_period" view
CREATE
OR REPLACE VIEW "current_billing_period" (
   "subscription_id",
   "current_period_start_date",
   "current_period_end_date",
   "current_period_idx"
) AS WITH billing_period_months AS (
   SELECT
      subscription.id AS subscription_id,
      subscription.billing_start_date,
      subscription.effective_billing_period,
      CASE
         subscription.effective_billing_period
         WHEN 'MONTHLY' :: "BillingPeriodEnum" THEN 1
         WHEN 'QUARTERLY' :: "BillingPeriodEnum" THEN 3
         WHEN 'ANNUAL' :: "BillingPeriodEnum" THEN 12
         ELSE NULL :: integer
      END AS months_per_period
   FROM
      subscription
),
billing_periods AS (
   SELECT
      billing_period_months.subscription_id,
      billing_period_months.billing_start_date,
      billing_period_months.effective_billing_period,
      billing_period_months.months_per_period,
      (
         (
            (
               (
                  EXTRACT(
                     year
                     FROM
                        age(
                           now(),
                           (billing_period_months.billing_start_date) :: timestamp with time zone
                        )
                  ) * (12) :: numeric
               ) + EXTRACT(
                  month
                  FROM
                     age(
                        now(),
                        (billing_period_months.billing_start_date) :: timestamp with time zone
                     )
               )
            ) - (1) :: numeric
         ) / (billing_period_months.months_per_period) :: numeric
      ) AS current_period_idx
   FROM
      billing_period_months
   WHERE
      (
         billing_period_months.months_per_period IS NOT NULL
      )
)
SELECT
   billing_periods.subscription_id,
   (
      billing_periods.billing_start_date + (
         (
            (
               billing_periods.current_period_idx * (billing_periods.months_per_period) :: numeric
            ) || ' months' :: text
         )
      ) :: interval
   ) AS current_period_start_date,
   (
      (
         billing_periods.billing_start_date + (
            (
               (
                  (
                     billing_periods.current_period_idx + (1) :: numeric
                  ) * (billing_periods.months_per_period) :: numeric
               ) || ' months' :: text
            )
         ) :: interval
      ) - '1 day' :: interval
   ) AS current_period_end_date,
   billing_periods.current_period_idx
FROM
   billing_periods;

-- Drop enum type "BillingFrequencyEnum"
DROP TYPE "BillingFrequencyEnum";
