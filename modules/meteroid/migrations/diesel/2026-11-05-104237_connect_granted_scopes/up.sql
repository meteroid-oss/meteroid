-- What the organization agreed to when it connected. Empty for express accounts, which the
-- platform owns.
ALTER TABLE connect_connected_account
    ADD COLUMN granted_scopes TEXT[] NOT NULL DEFAULT '{}';

-- Existing standard accounts keep the full access they had before scopes were enforced.
UPDATE connect_connected_account
SET granted_scopes = '{connect:full}'
WHERE connection_type = 'STANDARD';
