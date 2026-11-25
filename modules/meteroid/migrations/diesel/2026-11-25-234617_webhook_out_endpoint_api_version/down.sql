SET LOCAL lock_timeout = '5s';

ALTER TABLE webhook_out_endpoint DROP COLUMN api_version;
