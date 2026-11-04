DROP TABLE IF EXISTS "plan_version_annex_template";
DROP TABLE IF EXISTS "invoice_annex_template";
DROP TYPE IF EXISTS "InvoiceAnnexKindEnum";
ALTER TABLE "invoice" DROP COLUMN IF EXISTS "annexes";
