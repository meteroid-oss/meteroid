DELETE FROM api_token WHERE kind = 'PUBLISHABLE';

DROP INDEX api_token_hash_key;
CREATE UNIQUE INDEX api_token_hash_key ON api_token (hash);

ALTER TABLE api_token
    DROP CONSTRAINT api_token_kind_check,
    DROP COLUMN publishable_key,
    DROP COLUMN allowed_origins,
    DROP COLUMN kind;

DROP TYPE "ApiTokenKindEnum";
