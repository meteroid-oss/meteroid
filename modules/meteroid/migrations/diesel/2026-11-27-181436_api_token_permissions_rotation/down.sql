-- Without these columns every key has full access and never expires: drop the keys that would
-- otherwise come back to life, and those restricted.
DELETE FROM api_token WHERE expires_at IS NOT NULL OR permissions IS NOT NULL;

DROP INDEX IF EXISTS api_token_replaced_by_id_idx;
ALTER TABLE api_token
    DROP COLUMN replaced_by_id,
    DROP COLUMN lifetime_days,
    DROP COLUMN expires_at,
    DROP COLUMN permissions;
