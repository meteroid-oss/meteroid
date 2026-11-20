ALTER TYPE "StoredObjectKindEnum" ADD VALUE IF NOT EXISTS 'QUOTE_PDF';

-- The revision's document as a PDF, once rendered.
ALTER TABLE quote_revision ADD COLUMN pdf_document_id UUID;

-- Rows issued before the signed PDF existed hold a standalone certificate.
ALTER TABLE quote_signature_certificate
    ADD COLUMN document_included BOOLEAN NOT NULL DEFAULT false;
