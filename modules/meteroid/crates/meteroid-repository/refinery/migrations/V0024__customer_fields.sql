-- Modify "customer" table
ALTER TABLE "customer"
    ADD COLUMN "email" text,
    ADD COLUMN "invoicing_email" text,
    ADD COLUMN "phone" text,
    ADD COLUMN "balance_value_cents" INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN "balance_currency" text NOT NULL DEFAULT 'USD',
    ADD COLUMN "billing_address" JSONB,
    ADD COLUMN "shipping_address" JSONB;
