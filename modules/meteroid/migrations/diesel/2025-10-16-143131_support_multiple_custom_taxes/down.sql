-- Revert customer changes
ALTER TABLE customer
    DROP COLUMN custom_taxes,
    ADD COLUMN custom_tax_rate NUMERIC(7,4);

-- Re-add custom_tax_id to product_accounting
ALTER TABLE product_accounting ADD COLUMN custom_tax_id UUID REFERENCES custom_tax(id) ON DELETE SET NULL;

-- Migrate back (taking the first tax if multiple exist)
UPDATE product_accounting pa
SET custom_tax_id = pct.custom_tax_id
FROM (
    SELECT DISTINCT ON (product_id, invoicing_entity_id)
           product_id, invoicing_entity_id, custom_tax_id
    FROM product_custom_tax
    ORDER BY product_id, invoicing_entity_id, custom_tax_id
) pct
WHERE pa.product_id = pct.product_id
  AND pa.invoicing_entity_id = pct.invoicing_entity_id;

-- Drop join table
DROP TABLE product_custom_tax;
