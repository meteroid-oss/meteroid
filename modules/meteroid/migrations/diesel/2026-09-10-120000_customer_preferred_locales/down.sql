ALTER TABLE customer
    ADD COLUMN invoicing_language TEXT;

UPDATE customer
SET invoicing_language = preferred_locales[1]
WHERE cardinality(preferred_locales) > 0;

ALTER TABLE customer
    DROP COLUMN preferred_locales;
