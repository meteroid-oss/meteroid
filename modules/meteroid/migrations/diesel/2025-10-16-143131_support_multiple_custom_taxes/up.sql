-- Create join table for product to custom_tax
CREATE TABLE product_custom_tax (
    product_id UUID NOT NULL REFERENCES product(id) ON DELETE CASCADE,
    invoicing_entity_id UUID NOT NULL REFERENCES invoicing_entity(id) ON DELETE CASCADE,
    custom_tax_id UUID NOT NULL REFERENCES custom_tax(id) ON DELETE CASCADE,
    PRIMARY KEY (product_id, invoicing_entity_id, custom_tax_id)
);

-- Migrate existing data from product_accounting.custom_tax_id to the join table
INSERT INTO product_custom_tax (product_id, invoicing_entity_id, custom_tax_id)
SELECT product_id, invoicing_entity_id, custom_tax_id
FROM product_accounting
WHERE custom_tax_id IS NOT NULL;

-- Remove custom_tax_id from product_accounting
ALTER TABLE product_accounting DROP COLUMN custom_tax_id;

-- Update customer to support multiple custom taxes
-- Structure: [{"tax_code": "GST", "name": "Goods and Services Tax", "rate": 0.05}, ...]
ALTER TABLE customer ADD COLUMN custom_taxes JSONB NOT NULL DEFAULT '[]';
-- Migrate existing custom_tax_rate to custom_taxes array
UPDATE customer
SET custom_taxes = jsonb_build_array(
    jsonb_build_object(
        'tax_code', 'CUSTOM',
        'name', 'Custom Tax',
        'rate', custom_tax_rate
    )
)
WHERE custom_tax_rate IS NOT NULL;
-- drop the old column
ALTER TABLE customer DROP COLUMN custom_tax_rate;
