ALTER TABLE "customer"
  DROP COLUMN IF EXISTS "bank_account_id",
  DROP COLUMN IF EXISTS "current_payment_method_id",
  DROP COLUMN IF EXISTS "default_psp_connection_id",
  DROP COLUMN IF EXISTS "vat_number",
  DROP COLUMN IF EXISTS "custom_vat_rate",
  DROP COLUMN IF EXISTS "invoicing_emails",
  ADD COLUMN IF NOT EXISTS billing_config  jsonb not null default '{}',
  ADD COLUMN IF NOT EXISTS invoicing_email text;

ALTER TABLE "customer"
  RENAME COLUMN "billing_email" TO "email";

DROP TABLE IF EXISTS "customer_payment_method" CASCADE;

DROP TABLE IF EXISTS "payment_transaction" CASCADE;

DROP TABLE IF EXISTS "customer_connection" CASCADE;

ALTER TABLE "subscription"
  DROP COLUMN IF EXISTS "psp_connection_id",
  DROP COLUMN IF EXISTS "pending_checkout",
  DROP COLUMN IF EXISTS "payment_method",
  DROP COLUMN IF EXISTS "payment_method_type",
  DROP COLUMN IF EXISTS "end_date",
  DROP COLUMN IF EXISTS "trial_duration",
  DROP COLUMN IF EXISTS "activation_condition",
  DROP COLUMN IF EXISTS "billing_start_date"
;


ALTER TABLE "subscription"
  RENAME billing_day_anchor TO billing_day;
ALTER TABLE "subscription"
  RENAME start_date TO billing_start_date;

ALTER TABLE "subscription"
  ADD COLUMN "trial_start_date" DATE,
  ADD COLUMN "billing_end_date" DATE;

DROP TYPE IF EXISTS "PaymentStatusEnum";
DROP TYPE IF EXISTS "PaymentTypeEnum";
DROP TYPE IF EXISTS "PaymentMethodTypeEnum";
DROP TYPE IF EXISTS "SubscriptionActivationConditionEnum";
