-- Meteroid Connect: OAuth 2.0 Server + Connect Platform
-- This migration creates tables for:
-- - OAuth applications (registered by platforms)
-- - Authorization codes (short-lived, for OAuth code flow)
-- - Access tokens and refresh tokens
-- - Connected accounts (platform <-> organization relationships)
-- - Platform settings
-- - Express onboarding links

-- ============================================================================
-- Organization: add is_express flag
-- Express organizations are created via Connect Express flow and don't have direct user accounts
-- ============================================================================
ALTER TABLE organization
    ADD COLUMN is_express BOOLEAN NOT NULL DEFAULT FALSE;

-- ============================================================================
-- Enum types for Connect
-- ============================================================================
CREATE TYPE "ConnectConnectionTypeEnum" AS ENUM ('STANDARD', 'EXPRESS');
CREATE TYPE "ConnectConnectionStatusEnum" AS ENUM ('PENDING', 'ACTIVE', 'REVOKED', 'SUSPENDED');
CREATE TYPE "ConnectOnboardingModeEnum" AS ENUM ('EXPRESS', 'FULL');

-- ============================================================================
-- OAuth Applications (registered by platforms)
-- ============================================================================
CREATE TABLE connect_oauth_app (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organization(id),
    name TEXT NOT NULL,
    client_id TEXT NOT NULL UNIQUE,
    client_secret_hash TEXT NOT NULL,
    client_secret_hint TEXT NOT NULL,
    redirect_uris TEXT[] NOT NULL,
    scopes TEXT[] NOT NULL DEFAULT '{"connect:full"}',
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ
);

-- client_id already has an index from the UNIQUE constraint
CREATE INDEX idx_connect_oauth_app_organization ON connect_oauth_app(organization_id);

-- ============================================================================
-- Authorization Codes (short-lived, for OAuth code exchange)
-- ============================================================================
CREATE TABLE connect_authorization_code (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code_hash TEXT NOT NULL UNIQUE,
    oauth_app_id UUID NOT NULL REFERENCES connect_oauth_app(id) ON DELETE CASCADE,
    connected_organization_id UUID NOT NULL REFERENCES organization(id),
    redirect_uri TEXT NOT NULL,
    scopes TEXT[] NOT NULL,
    code_challenge TEXT,
    code_challenge_method TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    used_at TIMESTAMPTZ
);

-- code_hash already has an index from the UNIQUE constraint
CREATE INDEX idx_connect_auth_code_expires ON connect_authorization_code(expires_at);
CREATE INDEX idx_connect_auth_code_oauth_app ON connect_authorization_code(oauth_app_id);

-- ============================================================================
-- Access Tokens
-- ============================================================================
CREATE TABLE connect_access_token (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    token_hash TEXT NOT NULL UNIQUE,
    oauth_app_id UUID NOT NULL REFERENCES connect_oauth_app(id) ON DELETE CASCADE,
    connected_organization_id UUID NOT NULL REFERENCES organization(id),
    scopes TEXT[] NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);

-- token_hash already has an index from the UNIQUE constraint
CREATE INDEX idx_connect_access_token_expires ON connect_access_token(expires_at);
CREATE INDEX idx_connect_access_token_oauth_app ON connect_access_token(oauth_app_id);
CREATE INDEX idx_connect_access_token_org ON connect_access_token(connected_organization_id);

-- ============================================================================
-- Refresh Tokens
-- ============================================================================
CREATE TABLE connect_refresh_token (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    token_hash TEXT NOT NULL UNIQUE,
    access_token_id UUID NOT NULL REFERENCES connect_access_token(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    rotated_to_id UUID REFERENCES connect_refresh_token(id)
);

-- token_hash already has an index from the UNIQUE constraint
CREATE INDEX idx_connect_refresh_token_access ON connect_refresh_token(access_token_id);

-- ============================================================================
-- Connected Accounts (platform <-> organization relationships)
-- connected_organization_id is nullable for express accounts (created before org exists)
-- ============================================================================
CREATE TABLE connect_connected_account (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    platform_organization_id UUID NOT NULL REFERENCES organization(id),
    connected_organization_id UUID REFERENCES organization(id),
    connection_type "ConnectConnectionTypeEnum" NOT NULL,
    status "ConnectConnectionStatusEnum" NOT NULL DEFAULT 'PENDING',
    onboarding_mode "ConnectOnboardingModeEnum" NOT NULL DEFAULT 'EXPRESS',
    onboarding_completed_at TIMESTAMPTZ,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    -- Express flow pending info
    pending_email TEXT,
    pending_organization_name TEXT,
    pending_country TEXT,
    platform_customer_id UUID UNIQUE REFERENCES customer(id),
    connected_tenant_id UUID REFERENCES tenant(id),
    UNIQUE(platform_organization_id, connected_organization_id)
);

-- Standard accounts require connected_organization_id, express pending accounts don't
ALTER TABLE connect_connected_account
    ADD CONSTRAINT chk_express_or_connected CHECK (
        (connection_type = 'STANDARD' AND connected_organization_id IS NOT NULL) OR
        (connection_type = 'EXPRESS' AND (
            (status = 'PENDING' AND connected_organization_id IS NULL AND pending_email IS NOT NULL) OR
            (status != 'PENDING' AND connected_organization_id IS NOT NULL)
        ))
    );

CREATE INDEX idx_connect_connected_account_platform ON connect_connected_account(platform_organization_id);
CREATE INDEX idx_connect_connected_account_connected ON connect_connected_account(connected_organization_id);
CREATE INDEX idx_connect_connected_account_status ON connect_connected_account(status);
CREATE INDEX idx_connect_connected_account_pending_email
    ON connect_connected_account(pending_email)
    WHERE pending_email IS NOT NULL;

CREATE UNIQUE INDEX idx_connect_connected_account_pending_unique
    ON connect_connected_account(platform_organization_id, pending_email)
    WHERE status = 'PENDING';

ALTER TABLE customer
    ADD COLUMN connected_account_id UUID UNIQUE REFERENCES connect_connected_account(id);

-- ============================================================================
-- Platform Settings (configuration for organizations acting as platforms)
-- ============================================================================
CREATE TABLE connect_platform_settings (
    organization_id UUID PRIMARY KEY REFERENCES organization(id),
    is_platform_enabled BOOLEAN NOT NULL DEFAULT false,
    default_scopes TEXT[] NOT NULL DEFAULT '{"connect:full"}',
    branding_invoicing_entity_id UUID REFERENCES invoicing_entity(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- Express Onboarding Links (for express flow)
-- ============================================================================
CREATE TABLE connect_onboarding_link (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    connected_account_id UUID NOT NULL REFERENCES connect_connected_account(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    redirect_url TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- token_hash already has an index from the UNIQUE constraint
CREATE INDEX idx_connect_onboarding_link_expires ON connect_onboarding_link(expires_at);
CREATE INDEX idx_connect_onboarding_link_account ON connect_onboarding_link(connected_account_id);
