-- Modify "invoice" table
ALTER TABLE "invoice"
    DROP COLUMN "start_date",
    DROP COLUMN "end_date",
    ADD COLUMN "invoice_date" date NOT NULL;
