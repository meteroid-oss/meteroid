CREATE TYPE "PortalResourceKindEnum" AS ENUM ('INVOICE', 'QUOTE', 'CUSTOMER', 'CHECKOUT_SESSION', 'DASHBOARD');

-- Customer-facing links carry an opaque secret; only sha256(secret) is stored.
CREATE TABLE portal_token (
    id               UUID PRIMARY KEY,
    tenant_id        UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    token_hash       TEXT NOT NULL,
    resource_kind    "PortalResourceKindEnum" NOT NULL,
    resource_id      UUID NOT NULL,
    recipient_email  TEXT,
    scopes           TEXT[] NOT NULL DEFAULT '{}',
    delegated        BOOLEAN NOT NULL DEFAULT false,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at       TIMESTAMPTZ,
    revoked_at       TIMESTAMPTZ,
    last_used_at     TIMESTAMPTZ,
    last_recovery_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX portal_token_hash_idx ON portal_token (token_hash);
CREATE INDEX portal_token_resource_idx ON portal_token (tenant_id, resource_kind, resource_id);

-- Shares now resolve through portal_token; the column only keeps pre-existing JWTs.
ALTER TABLE analytics_dashboard_share ALTER COLUMN token DROP NOT NULL;
