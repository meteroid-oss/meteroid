-- Registry of every stored object's tenant, kind, and resolved object-store key, so a tenant's
-- objects can be enumerated, read, and deleted exactly from one place.
CREATE TYPE "StoredObjectKindEnum" AS ENUM (
    'INVOICE_PDF',
    'INVOICE_XML',
    'CREDIT_NOTE_PDF',
    'RECEIPT_PDF',
    'IMAGE_LOGO',
    'WEBHOOK_ARCHIVE',
    'BATCH_JOB_INPUT',
    'BATCH_JOB_ERROR_OUTPUT',
    'TAX_REPORT'
);

CREATE TABLE stored_object (
    id         UUID PRIMARY KEY,
    tenant_id  UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    kind       "StoredObjectKindEnum" NOT NULL,
    -- Relative object-store key (base path prepended at read time). A legacy operational key keeps
    -- the `%2F`-joined segment the old layout wrote, so it resolves to the exact existing object.
    key        TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_stored_object_tenant ON stored_object (tenant_id);

-- base62 (BaseId::as_base62) <-> UUID helpers. Operational objects store their id as a base62
-- string, and their legacy key embeds the tenant's base62 id. Both are dropped again below.
CREATE OR REPLACE FUNCTION _meteroid_base62_to_uuid(encoded TEXT) RETURNS UUID AS $$
DECLARE
    alphabet CONSTANT TEXT := '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
    payload  TEXT;
    val      NUMERIC := 0;
    i        INT;
    c        CHAR;
    pos      INT;
    pow67    CONSTANT NUMERIC := 147573952589676412928;
    pow61    CONSTANT NUMERIC := 2305843009213693952;
    pow128   CONSTANT NUMERIC := 340282366920938463463374607431768211456;
    rotated  NUMERIC;
    hex_str  TEXT := '';
    rem      INT;
    hex_chars CONSTANT TEXT := '0123456789abcdef';
BEGIN
    payload := regexp_replace(encoded, '^.*_', '');
    FOR i IN 1..length(payload) LOOP
        c   := substr(payload, i, 1);
        pos := position(c IN alphabet) - 1;
        IF pos < 0 THEN
            RAISE EXCEPTION '_meteroid_base62_to_uuid: invalid char % in %', c, encoded;
        END IF;
        val := val * 62 + pos;
    END LOOP;
    rotated := (div(val, pow67) + ((val % pow67) * pow61)) % pow128;
    FOR i IN 1..32 LOOP
        rem     := (rotated % 16)::INT;
        hex_str := substr(hex_chars, rem + 1, 1) || hex_str;
        rotated := div(rotated, 16);
    END LOOP;
    RETURN (
        substr(hex_str,  1, 8) || '-' ||
        substr(hex_str,  9, 4) || '-' ||
        substr(hex_str, 13, 4) || '-' ||
        substr(hex_str, 17, 4) || '-' ||
        substr(hex_str, 21, 12)
    )::UUID;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE OR REPLACE FUNCTION _meteroid_uuid_to_base62(u UUID) RETURNS TEXT AS $$
DECLARE
    alphabet CONSTANT TEXT    := '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
    pow67    CONSTANT NUMERIC := 147573952589676412928;
    pow61    CONSTANT NUMERIC := 2305843009213693952;
    hex_str  TEXT;
    val      NUMERIC;
    rotated  NUMERIC;
    rem      INT;
    encoded  TEXT := '';
    i        INT;
BEGIN
    hex_str := replace(u::TEXT, '-', '');
    val := 0;
    FOR i IN 1..32 LOOP
        val := val * 16 + (position(substr(hex_str, i, 1) IN '0123456789abcdef') - 1);
    END LOOP;
    rotated := (val % pow61) * pow67 + div(val, pow61);
    IF rotated = 0 THEN
        encoded := '0';
    ELSE
        WHILE rotated > 0 LOOP
            rem     := (rotated % 62)::INT;
            encoded := substr(alphabet, rem + 1, 1) || encoded;
            rotated := div(rotated, 62);
        END LOOP;
    END IF;
    RETURN encoded;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Backfill every existing object with its legacy key. Business objects were tenant-less
-- (`{type}/{uuid}`); operational objects embedded the tenant (and, for webhooks, the alias) in a
-- single `%2F`-joined segment. The first union branch casts the kind literal so the column resolves
-- to the enum type rather than text.
INSERT INTO stored_object (id, tenant_id, kind, key)
SELECT pdf_document_id, tenant_id, 'INVOICE_PDF'::"StoredObjectKindEnum",
       'invoice_pdf/' || pdf_document_id::text
    FROM invoice WHERE pdf_document_id IS NOT NULL
UNION ALL
SELECT xml_document_id, tenant_id, 'INVOICE_XML',
       'invoice_xml/' || xml_document_id::text
    FROM invoice WHERE xml_document_id IS NOT NULL
UNION ALL
SELECT pdf_document_id, tenant_id, 'CREDIT_NOTE_PDF',
       'credit_note_pdf/' || pdf_document_id::text
    FROM credit_note WHERE pdf_document_id IS NOT NULL
UNION ALL
SELECT logo_attachment_id, tenant_id, 'IMAGE_LOGO',
       'image_logo/' || logo_attachment_id::text
    FROM invoicing_entity WHERE logo_attachment_id IS NOT NULL
UNION ALL
SELECT receipt_pdf_id, tenant_id, 'RECEIPT_PDF',
       'receipt_pdf/' || receipt_pdf_id::text
    FROM payment_transaction WHERE receipt_pdf_id IS NOT NULL
UNION ALL
SELECT _meteroid_base62_to_uuid(input_source_key), tenant_id, 'BATCH_JOB_INPUT',
       'batch_job%2Ften_' || _meteroid_uuid_to_base62(tenant_id) || '/'
           || _meteroid_base62_to_uuid(input_source_key)::text
    FROM batch_job WHERE input_source_key IS NOT NULL
UNION ALL
SELECT _meteroid_base62_to_uuid(error_output_key), tenant_id, 'BATCH_JOB_ERROR_OUTPUT',
       'batch_job_errors%2Ften_' || _meteroid_uuid_to_base62(tenant_id) || '/'
           || _meteroid_base62_to_uuid(error_output_key)::text
    FROM batch_job WHERE error_output_key IS NOT NULL
UNION ALL
SELECT _meteroid_base62_to_uuid(output_key), tenant_id, 'TAX_REPORT',
       'tax_report%2Ften_' || _meteroid_uuid_to_base62(tenant_id) || '/'
           || _meteroid_base62_to_uuid(output_key)::text
    FROM batch_job WHERE output_key IS NOT NULL
ON CONFLICT (id) DO NOTHING;

-- Webhook archives resolve their tenant + connection alias through the connector.
INSERT INTO stored_object (id, tenant_id, kind, key)
SELECT wie.id, c.tenant_id, 'WEBHOOK_ARCHIVE'::"StoredObjectKindEnum",
       'webhook_archive%2Ften_' || _meteroid_uuid_to_base62(c.tenant_id) || '%2F' || c.alias
           || '/' || wie.id::text
    FROM webhook_in_event wie
    JOIN connector c ON c.id = wie.provider_config_id
ON CONFLICT (id) DO NOTHING;

DROP FUNCTION _meteroid_base62_to_uuid(TEXT);
DROP FUNCTION _meteroid_uuid_to_base62(UUID);
