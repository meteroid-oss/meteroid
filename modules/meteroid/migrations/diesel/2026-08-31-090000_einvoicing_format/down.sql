ALTER TABLE invoicing_entity
    DROP COLUMN IF EXISTS einvoicing_format,
    DROP COLUMN IF EXISTS legal_number;

DROP TYPE IF EXISTS "EInvoicingFormatEnum";
