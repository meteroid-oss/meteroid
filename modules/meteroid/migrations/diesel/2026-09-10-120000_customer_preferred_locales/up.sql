-- Customer document-language preferences, most-preferred first, in the Stripe
-- `preferred_locales` shape (["en", "fr-FR"]). An empty list means "inherit from
-- the invoicing entity", which is what the single nullable column meant before.
ALTER TABLE customer
    ADD COLUMN preferred_locales TEXT[] NOT NULL DEFAULT '{}';

UPDATE customer
SET preferred_locales = ARRAY [btrim(invoicing_language)]
WHERE invoicing_language IS NOT NULL
  AND btrim(invoicing_language) <> '';

ALTER TABLE customer
    DROP COLUMN invoicing_language;
