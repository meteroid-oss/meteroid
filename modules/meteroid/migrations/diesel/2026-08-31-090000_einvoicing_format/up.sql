-- Structured e-invoicing, opt-in per invoicing entity.
--
-- The three values are siblings, not levels: FACTUR_X is the EN 16931 core
-- model carried inside the PDF, XRECHNUNG and PEPPOL_BIS are CIUSes of it that
-- each forbid what the other allows. NULL (the default) means the entity issues
-- PDFs only, exactly as before.
CREATE TYPE "EInvoicingFormatEnum" AS ENUM ('FACTUR_X', 'XRECHNUNG', 'PEPPOL_BIS');

ALTER TABLE invoicing_entity
    ADD COLUMN einvoicing_format "EInvoicingFormatEnum",
    -- BT-30: the seller's registration in its national register (SIREN/SIRET,
    -- Handelsregisternummer, ...). Distinct from the VAT number, which not every
    -- seller has and which identifies a different thing.
    ADD COLUMN legal_number TEXT;
