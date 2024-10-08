ALTER TABLE "invoice"
  DROP COLUMN "xml_document_id";
ALTER TABLE "invoice"
  DROP COLUMN "pdf_document_id";

DROP TABLE IF EXISTS "outbox";
