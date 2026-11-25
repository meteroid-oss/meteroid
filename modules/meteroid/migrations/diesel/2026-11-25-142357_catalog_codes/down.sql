ALTER TABLE api_idempotency_key
    DROP COLUMN response_status,
    DROP COLUMN response_body;

DROP INDEX IF EXISTS subscription_tenant_external_id_key;
ALTER TABLE subscription DROP COLUMN external_id;

DROP TRIGGER IF EXISTS trg_plan_fill_code ON plan;
DROP TRIGGER IF EXISTS trg_product_fill_code ON product;
DROP TRIGGER IF EXISTS trg_add_on_fill_code ON add_on;
DROP TRIGGER IF EXISTS trg_product_family_fill_code ON product_family;

ALTER TABLE plan DROP COLUMN code;
ALTER TABLE product DROP COLUMN code;
ALTER TABLE add_on DROP COLUMN code;
ALTER TABLE product_family DROP COLUMN code;

DROP FUNCTION IF EXISTS fill_catalog_code();
DROP FUNCTION IF EXISTS unique_catalog_code(REGCLASS, UUID, TEXT);
DROP FUNCTION IF EXISTS catalog_code_lock(REGCLASS, UUID, TEXT);
DROP FUNCTION IF EXISTS catalog_code_slug(TEXT, TEXT);
