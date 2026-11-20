-- The QUOTE_PDF value of "StoredObjectKindEnum" stays: Postgres cannot remove an enum value.
ALTER TABLE quote_signature_certificate DROP COLUMN document_included;
ALTER TABLE quote_revision DROP COLUMN pdf_document_id;
