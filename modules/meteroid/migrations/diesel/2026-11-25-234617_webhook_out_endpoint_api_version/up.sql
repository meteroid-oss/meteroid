-- The API version an endpoint's payloads are rendered at (ADR-0013 §15): set from the
-- `Meteroid-Version` of the v2 request that created it. NULL on an endpoint created through v1,
-- the dashboard or before versions were recorded: v1's payloads.
SET LOCAL lock_timeout = '5s';

ALTER TABLE webhook_out_endpoint ADD COLUMN api_version TEXT;
