SET LOCAL lock_timeout = '5s';

ALTER TABLE webhook_out_delivery DROP CONSTRAINT webhook_out_delivery_message_version_fkey;
ALTER TABLE webhook_out_delivery DROP COLUMN api_version;
DROP TABLE webhook_out_message_version;
