ALTER TABLE organization ADD COLUMN IF NOT EXISTS invite_link_hash TEXT UNIQUE;

DROP TABLE IF EXISTS organization_invite;
