-- Emails are stored trimmed and lower-cased from now on. Accounts that only differ by case
-- from another one keep their spelling: merging them is not ours to decide.
UPDATE "user" u
SET email = lower(btrim(u.email))
WHERE u.email <> lower(btrim(u.email))
  AND NOT EXISTS (
      SELECT 1 FROM "user" o
      WHERE o.id <> u.id AND lower(btrim(o.email)) = lower(btrim(u.email))
  );

CREATE INDEX user_email_lower_idx ON "user" (lower(email));

-- Sessions carry the epoch they were issued in; a password reset bumps it, ending them all.
ALTER TABLE "user" ADD COLUMN session_epoch INTEGER NOT NULL DEFAULT 0;

-- Single-use reset links: only sha256(secret) is stored.
CREATE TABLE password_reset_token (
    id          UUID        PRIMARY KEY,
    user_id     UUID        NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    token_hash  TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ NULL
);

CREATE UNIQUE INDEX password_reset_token_hash_idx ON password_reset_token (token_hash);
CREATE INDEX password_reset_token_user_open_idx ON password_reset_token (user_id)
    WHERE consumed_at IS NULL;

-- Verified domains are re-checked periodically; ownership is dropped after a grace period
-- of failed checks.
ALTER TABLE organization_domain
    ADD COLUMN last_checked_at TIMESTAMPTZ NULL,
    ADD COLUMN check_failing_since TIMESTAMPTZ NULL;

-- Existing domains are first re-checked a full period after their verification.
UPDATE organization_domain SET last_checked_at = verified_at WHERE verified_at IS NOT NULL;
