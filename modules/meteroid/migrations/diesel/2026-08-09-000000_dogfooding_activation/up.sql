-- Feeds the first_api_key_used activation milestone and the API keys tab.
ALTER TABLE api_token
    ADD COLUMN first_used_at TIMESTAMP(3),
    ADD COLUMN last_used_at  TIMESTAMP(3);

-- Labels are the SCREAMING form of Milestone::as_str() (lower(label) feeds
-- stable event ids); new milestones need ALTER TYPE "MilestoneEnum" ADD VALUE.
CREATE TYPE "MilestoneEnum" AS ENUM (
    'FIRST_CUSTOMER_CREATED',
    'FIRST_BILLABLE_METRIC_CREATED',
    'FIRST_PLAN_CREATED',
    'FIRST_PLAN_PUBLISHED',
    'FIRST_API_KEY_CREATED',
    'FIRST_API_KEY_USED',
    'FIRST_EVENT_INGESTED',
    'FIRST_SUBSCRIPTION_CREATED',
    'FIRST_INVOICE_FINALIZED',
    'FIRST_ADDON_CREATED',
    'FIRST_FEATURE_CREATED',
    'FIRST_QUOTE_CREATED'
);

-- First-occurrence activation milestones per organization.
-- Rows are write-once: the derivation worker inserts with ON CONFLICT DO NOTHING.
CREATE TABLE organization_milestone (
    organization_id UUID            NOT NULL REFERENCES organization (id) ON DELETE CASCADE,
    milestone       "MilestoneEnum" NOT NULL,
    occurred_at     TIMESTAMP(3) NOT NULL,
    -- Tenant that first achieved the milestone, when tenant-scoped.
    tenant_id       UUID REFERENCES tenant (id) ON DELETE SET NULL,
    created_at      TIMESTAMP(3) NOT NULL DEFAULT now(),
    -- Sync-cursor tie-breaker; created_at is shared by a whole derivation batch.
    id              UUID         NOT NULL DEFAULT uuidv7(),
    -- First occurrence in a PRODUCTION tenant; NULL = non-prod only so far.
    production_occurred_at TIMESTAMP(3),
    PRIMARY KEY (organization_id, milestone)
);

-- Per-stream cursor for the activation event sync.
-- Deleting a row re-syncs that stream; stable event ids make re-runs safe.
CREATE TABLE dogfooding_sync_state (
    stream     TEXT PRIMARY KEY,
    cursor_at  TIMESTAMP(6) NOT NULL,
    cursor_id  UUID         NOT NULL,
    updated_at TIMESTAMP(3) NOT NULL DEFAULT now()
);

-- Partial index for the user_connection stream's incremental scan.
CREATE INDEX idx_org_security_audit_log_login_sync
    ON org_security_audit_log (occurred_at, id)
    WHERE event_type IN ('auth.login.succeeded', 'auth.oauth.login.succeeded', 'auth.sso.login.succeeded');

-- The milestones activation stream pages by (created_at, id).
CREATE INDEX idx_organization_milestone_sync ON organization_milestone (created_at, id);

-- Cursor indexes for the remaining activation streams.
CREATE INDEX idx_organization_created_sync ON organization (created_at, id);
CREATE INDEX idx_api_token_created_sync ON api_token (created_at, id);
CREATE INDEX idx_organization_invite_created_sync ON organization_invite (created_at, id);
CREATE INDEX idx_organization_invite_accepted_sync
    ON organization_invite (accepted_at, id)
    WHERE accepted_at IS NOT NULL;
