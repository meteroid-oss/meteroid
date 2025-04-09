SELECT pgmq.create('hubspot_sync');

ALTER TABLE customer ADD COLUMN conn_meta JSONB;
ALTER TABLE subscription ADD COLUMN conn_meta JSONB;
