-- Fails if an archived feature shares its code with another feature.
DROP INDEX feature_tenant_code_key;

ALTER TABLE feature
    ADD CONSTRAINT feature_tenant_code_key UNIQUE (tenant_id, code);
