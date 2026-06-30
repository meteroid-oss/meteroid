ALTER TABLE feature ADD COLUMN code TEXT;

UPDATE feature SET code = id::text WHERE code IS NULL;

ALTER TABLE feature ALTER COLUMN code SET NOT NULL;

ALTER TABLE feature ADD CONSTRAINT feature_tenant_code_key UNIQUE (tenant_id, code);

ALTER TABLE feature DROP CONSTRAINT IF EXISTS feature_tenant_id_name_key;
