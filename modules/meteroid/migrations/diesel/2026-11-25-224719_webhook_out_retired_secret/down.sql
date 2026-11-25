-- The newest secret still signing goes back to the endpoint's single `previous_secret`.
UPDATE webhook_out_endpoint e
SET previous_secret = r.secret, previous_secret_expires_at = r.expires_at
FROM (
    SELECT DISTINCT ON (endpoint_id) endpoint_id, secret, expires_at
    FROM webhook_out_retired_secret
    WHERE expires_at > (now() AT TIME ZONE 'UTC')
    ORDER BY endpoint_id, expires_at DESC
) r
WHERE e.id = r.endpoint_id;

DROP TABLE IF EXISTS webhook_out_retired_secret;
