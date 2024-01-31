ALTER TABLE customer DROP COLUMN aliases;
ALTER TABLE customer ADD COLUMN alias TEXT DEFAULT NULL;
CREATE UNIQUE INDEX customer_tenant_id_alias_idx ON customer (tenant_id, alias) NULLS NOT DISTINCT;