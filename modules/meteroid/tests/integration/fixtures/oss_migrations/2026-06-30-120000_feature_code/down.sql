ALTER TABLE feature
    DROP CONSTRAINT IF EXISTS feature_tenant_code_key;

ALTER TABLE feature
    DROP COLUMN IF EXISTS code;

ALTER TABLE feature
    ADD CONSTRAINT feature_tenant_id_name_key UNIQUE (tenant_id, name);
