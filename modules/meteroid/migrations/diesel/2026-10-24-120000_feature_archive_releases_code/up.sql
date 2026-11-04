-- Archiving releases the code, so a new feature can take the code of an archived one.
ALTER TABLE feature
    DROP CONSTRAINT feature_tenant_code_key;

CREATE UNIQUE INDEX feature_tenant_code_key
    ON feature (tenant_id, code)
    WHERE status <> 'ARCHIVED';
