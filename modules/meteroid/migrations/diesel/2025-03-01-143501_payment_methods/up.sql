create type "PaymentMethodTypeEnum" as enum ('CARD', 'TRANSFER', 'DIRECT_DEBIT_SEPA', 'DIRECT_DEBIT_ACH', 'DIRECT_DEBIT_BACS', 'OTHER');
create type "PaymentStatusEnum" as enum ('READY', 'PENDING', 'SETTLED', 'CANCELLED', 'FAILED');
create type "PaymentTypeEnum" as enum ('PAYMENT', 'REFUND');


-- a customer can have multiple connections to multiple providers
CREATE TABLE "customer_connection"
(
  "id"                      UUID NOT NULL PRIMARY KEY,
  "customer_id"             UUID NOT NULL REFERENCES "customer" ON DELETE CASCADE,
  "connector_id"            UUID NOT NULL REFERENCES "connector" ON DELETE CASCADE,
  "supported_payment_types" "PaymentMethodTypeEnum"[],
  "external_customer_id"    TEXT NOT NULL,
--   "external_account_id"     TEXT,
  unique ("customer_id", "connector_id")
);

CREATE TABLE "customer_payment_method"
(
  "id"                         UUID                    NOT NULL PRIMARY KEY,
  "tenant_id"                  UUID                    NOT NULL REFERENCES "tenant" ON DELETE RESTRICT,
  "customer_id"                UUID                    NOT NULL REFERENCES "customer" ON DELETE CASCADE,
  "connection_id"              UUID REFERENCES "customer_connection" ON DELETE CASCADE,
  "external_payment_method_id" TEXT,
  "created_at"                 TIMESTAMP               NOT NULL,
  "updated_at"                 TIMESTAMP               NOT NULL,
  "archived_at"                TIMESTAMP,
  "payment_method_type"        "PaymentMethodTypeEnum" NOT NULL,
  "account_number_hint"        TEXT,
  "card_brand"                 TEXT,
  "card_last4"                 TEXT,
  "card_exp_month"             INT4,
  "card_exp_year"              INT4
);
ALTER TABLE "customer_payment_method"
  -- nulls are distinct
  ADD UNIQUE ("connection_id", "external_payment_method_id");



CREATE TABLE "payment_transaction"
(
  "id"                      UUID                NOT NULL PRIMARY KEY,
  "tenant_id"               UUID                NOT NULL REFERENCES "tenant" ON DELETE RESTRICT,
  "invoice_id"              UUID                NOT NULL REFERENCES "invoice" ON DELETE RESTRICT,
  "provider_transaction_id" TEXT,
  "processed_at"            TIMESTAMP,
  "refunded_at"             TIMESTAMP,
  "amount"                  INT8                NOT NULL,
  "currency"                TEXT                NOT NULL,
  "payment_method_id"       UUID REFERENCES "customer_payment_method" ON DELETE RESTRICT,
  "status"                  "PaymentStatusEnum" NOT NULL,
  "payment_type"            "PaymentTypeEnum"   NOT NULL,
  "error_type"              TEXT
);


create type "SubscriptionActivationConditionEnum" as enum ('ON_START', 'ON_CHECKOUT', 'MANUAL');

ALTER TABLE "subscription"
  DROP COLUMN "trial_start_date";
ALTER TABLE "subscription"
  DROP COLUMN "billing_end_date";

ALTER TABLE "subscription"
  RENAME "billing_day" TO "billing_day_anchor";
ALTER TABLE "subscription"
  RENAME "billing_start_date" to "start_date";


ALTER TABLE "subscription"
  ADD COLUMN "psp_connection_id"    UUID                                  REFERENCES "customer_connection" ON DELETE SET NULL,
  ADD COLUMN "pending_checkout"     BOOLEAN                               NOT NULL DEFAULT false,
  ADD COLUMN payment_method_type    "PaymentMethodTypeEnum",
  ADD COLUMN "payment_method"       UUID                                  REFERENCES "customer_payment_method" ON DELETE SET NULL,
  ADD COLUMN "end_date"             DATE,
  ADD COLUMN "trial_duration"       INT4,
  ADD COLUMN "activation_condition" "SubscriptionActivationConditionEnum" NOT NULL DEFAULT 'MANUAL',
  ADD COLUMN "billing_start_date"   DATE;


-- a customer can define the active connection. As a second step we'll allow defining multiple connections based on the payment method
ALTER TABLE "customer"
  ADD COLUMN "bank_account_id"           UUID   REFERENCES "bank_account" ON DELETE SET NULL,
  ADD COLUMN "current_payment_method_id" UUID   REFERENCES "customer_payment_method" ON DELETE SET NULL,
  ADD COLUMN "default_psp_connection_id" UUID   REFERENCES "customer_connection" ON DELETE SET NULL,
  --   ADD COLUMN "card_provider_id" UUID REFERENCES "connector" ON DELETE SET NULL
  --   ADD COLUMN "sepa_provider_id" UUID REFERENCES "connector" ON DELETE SET NULL etc
  ADD COLUMN "vat_number"                TEXT,
  ADD COLUMN "custom_vat_rate"           INT4,
  ADD COLUMN "invoicing_emails"          TEXT[] NOT NULL DEFAULT '{}',
  DROP COLUMN billing_config,
  DROP COLUMN invoicing_email
;

ALTER TABLE "customer"
  RENAME COLUMN "email" TO "billing_email";
