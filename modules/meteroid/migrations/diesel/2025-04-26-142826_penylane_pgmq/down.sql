SELECT pgmq.drop_queue('pennylane_sync');

ALTER TABLE invoice DROP COLUMN conn_meta;
