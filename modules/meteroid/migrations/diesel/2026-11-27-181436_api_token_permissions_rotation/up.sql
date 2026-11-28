-- NULL permissions: full access, as every key had before.
ALTER TABLE api_token
    ADD COLUMN permissions JSONB,
    ADD COLUMN expires_at TIMESTAMP,
    ADD COLUMN lifetime_days INTEGER CHECK (lifetime_days > 0),
    ADD COLUMN replaced_by_id UUID REFERENCES api_token (id) ON DELETE SET NULL;

CREATE INDEX api_token_replaced_by_id_idx ON api_token (replaced_by_id) WHERE replaced_by_id IS NOT NULL;
