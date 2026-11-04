-- Per-language overrides for the invoice footer text. The existing
-- invoice_footer_info / invoice_footer_legal columns remain the default (untagged)
-- text; this holds locale-specific versions selected at render time by the
-- document's resolved language. Shape: { "<locale>": { "info": "...", "legal": "..." } }.
ALTER TABLE invoicing_entity
    ADD COLUMN invoice_footer_translations JSONB NOT NULL DEFAULT '{}'::jsonb;
