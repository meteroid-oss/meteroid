-- Drop index "invoice_external_invoice_id_key" from table: "invoice"
DROP INDEX "invoice_invoice_id_key";
DROP INDEX "invoice_external_invoice_id_key";
-- Modify "invoice" table
ALTER TABLE "invoice"
    RENAME COLUMN "amount_cents" TO "total";
ALTER TABLE "invoice"
    DROP COLUMN "days_until_due",
    DROP COLUMN "invoice_id",
    ADD COLUMN "net_terms"          integer NOT NULL,
--     ADD COLUMN "purchase_order"     text        NULL,
    ADD COLUMN "memo"               text    NULL,
    ADD COLUMN "tax_rate"           integer NOT NULL,
    ADD COLUMN "local_id"           text    NOT NULL,
    ADD COLUMN "reference"          text    NULL,
    ADD COLUMN "invoice_number"     text    NOT NULL,
    ADD COLUMN "tax_amount"         bigint  NOT NULL,
    ADD COLUMN "subtotal_recurring" bigint  NOT NULL,
    ADD COLUMN "plan_name"          text    NULL,
    ADD COLUMN "due_date"           date    NULL,
    ADD COLUMN "customer_details"   jsonb   NOT NULL,
    ADD COLUMN "amount_due"         bigint  NOT NULL,
    ADD COLUMN "subtotal"           bigint  NOT NULL,
    ADD CONSTRAINT "invoice_customer_id_fkey" FOREIGN KEY ("customer_id") REFERENCES "customer" ("id") ON UPDATE CASCADE ON DELETE RESTRICT,
    ADD CONSTRAINT "invoice_tenant_id_fkey" FOREIGN KEY ("tenant_id") REFERENCES "tenant" ("id") ON UPDATE CASCADE ON DELETE RESTRICT;
-- Create index "invoice_external_invoice_id_key" to table: "invoice"
CREATE UNIQUE INDEX "invoice_external_invoice_id_key" ON "invoice" ("external_invoice_id", "tenant_id");
-- Create index "invoice_invoice_number_key" to table: "invoice"

ALTER TABLE "subscription"
    ADD COLUMN "period" "BillingPeriodEnum" NOT NULL;

DROP INDEX "invoice_invoice_number_key";

CREATE UNIQUE INDEX "invoice_invoice_number_key" ON "invoice" ("invoice_number", "tenant_id") WHERE "invoice_number" IS NOT NULL;

