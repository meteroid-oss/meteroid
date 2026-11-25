-- Every signing secret an endpoint rotated away from signs beside its current one for 24 hours
-- (ADR-0013 §15), however often it is rotated in that window: one row per retired secret, sealed
-- with the store crypt key as the endpoint's `secret` is. The endpoint's single
-- `previous_secret` is moved here and no longer written (read only while a release still
-- rotating into it rolls out).
SET LOCAL lock_timeout = '5s';

CREATE TABLE webhook_out_retired_secret (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    endpoint_id UUID NOT NULL REFERENCES webhook_out_endpoint (id) ON DELETE CASCADE,
    secret TEXT NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT (now() AT TIME ZONE 'UTC')
);

CREATE INDEX webhook_out_retired_secret_endpoint_idx
    ON webhook_out_retired_secret (endpoint_id, expires_at);

INSERT INTO webhook_out_retired_secret (id, tenant_id, endpoint_id, secret, expires_at)
SELECT gen_random_uuid(), tenant_id, id, previous_secret, previous_secret_expires_at
FROM webhook_out_endpoint
WHERE previous_secret IS NOT NULL
  AND previous_secret_expires_at > (now() AT TIME ZONE 'UTC');

UPDATE webhook_out_endpoint
SET previous_secret = NULL, previous_secret_expires_at = NULL
WHERE previous_secret IS NOT NULL;
