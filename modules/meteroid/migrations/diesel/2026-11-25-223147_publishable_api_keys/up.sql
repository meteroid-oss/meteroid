-- Publishable keys (`pk_{env}_…`, docs/design/FRONTEND_SDK.md §5.3): an API key kind that only
-- reads the client API's public routes from a browser. A secret key is stored as its argon2 hash
-- and shown once; a publishable key is not a secret, so it is kept whole to be shown again, and has
-- no hash: `hash` stays empty, which no argon2 check accepts. The secret key validation reads
-- `kind = 'SECRET'` only, and only secret keys' hashes are unique: every publishable key's is empty.
SET LOCAL lock_timeout = '5s';

CREATE TYPE "ApiTokenKindEnum" AS ENUM ('SECRET', 'PUBLISHABLE');

ALTER TABLE api_token
    ADD COLUMN kind "ApiTokenKindEnum" NOT NULL DEFAULT 'SECRET',
    -- The browser origins (scheme://host[:port]) a publishable key is accepted from; none: any.
    ADD COLUMN allowed_origins TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN publishable_key TEXT,
    ADD CONSTRAINT api_token_kind_check CHECK (
        (kind = 'SECRET' AND publishable_key IS NULL AND allowed_origins = '{}')
        OR (kind = 'PUBLISHABLE' AND publishable_key IS NOT NULL AND hash = '')
    );

DROP INDEX api_token_hash_key;
CREATE UNIQUE INDEX api_token_hash_key ON api_token (hash) WHERE kind = 'SECRET';
