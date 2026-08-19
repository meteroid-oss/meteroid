-- A custom tax may target a tax category instead of being wired product by product:
-- it then applies to every line whose resolved category matches. NULL keeps the
-- existing behaviour (applies only to products explicitly linked via product_custom_tax).
ALTER TABLE custom_tax
    ADD COLUMN tax_category_id UUID REFERENCES tax_category(id) ON DELETE SET NULL;

CREATE INDEX custom_tax_category_idx
    ON custom_tax (invoicing_entity_id, tax_category_id)
    WHERE tax_category_id IS NOT NULL;
