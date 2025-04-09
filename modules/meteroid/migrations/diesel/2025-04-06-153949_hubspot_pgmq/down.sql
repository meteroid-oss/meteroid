SELECT pgmq.drop_queue('hubspot_sync');

ALTER TABLE customer DROP COLUMN conn_meta;
ALTER TABLE subscription DROP COLUMN conn_meta;
