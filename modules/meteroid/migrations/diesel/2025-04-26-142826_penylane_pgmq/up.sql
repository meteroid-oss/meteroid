SELECT pgmq.create('pennylane_sync');

ALTER TABLE invoice ADD COLUMN conn_meta JSONB;
