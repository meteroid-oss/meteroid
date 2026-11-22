ALTER TYPE "StoredObjectKindEnum" ADD VALUE IF NOT EXISTS 'QUOTE_ATTACHMENT';

-- An uploaded file, immutable: quotes and templates refer to it, and a quote's document hashes
-- its content.
CREATE TABLE quote_attachment (
    id          UUID PRIMARY KEY,
    tenant_id   UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    document_id UUID NOT NULL,
    filename    TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes  BIGINT NOT NULL,
    sha256      TEXT NOT NULL,
    created_by  UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX quote_attachment_tenant_idx ON quote_attachment (tenant_id);

-- No upload ever produced these ids; dropping them would change published documents' hashes.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM quote WHERE cardinality(attachments) > 0) THEN
        RAISE EXCEPTION 'quote.attachments holds ids from before uploads existed: clear them first';
    END IF;
END $$;

ALTER TABLE quote ALTER COLUMN attachments DROP DEFAULT;
ALTER TABLE quote ALTER COLUMN attachments TYPE JSONB USING '[]'::jsonb;
ALTER TABLE quote ALTER COLUMN attachments SET DEFAULT '[]'::jsonb;
