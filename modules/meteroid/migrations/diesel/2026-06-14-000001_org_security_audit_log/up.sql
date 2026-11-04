-- Organization-scoped security audit log.
--
-- Distinct from `entity_activity` (which is tenant-scoped business activity)
-- because security events — sign-ins, MFA changes, member invites, OAuth-app
-- grants — are org-wide and must survive tenant deletion for compliance
-- (SOC2 / ISO 27001 CC7.2). Entries are append-only at the application
-- layer; rotation is policy-driven (years, not days).
CREATE TABLE org_security_audit_log (
    id                  UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id     UUID            NOT NULL REFERENCES organization (id) ON DELETE CASCADE,

    -- Stable, dot-separated event identifier (e.g. "auth.login.succeeded").
    -- Free-form text so adding a new event type is a code-only change.
    event_type          TEXT            NOT NULL,

    actor_type          "ActorTypeEnum" NOT NULL,
    actor_id            UUID            NULL,
    -- Snapshotted at insert time. Keeps the trail readable after the user
    -- row is deleted or their email changes.
    actor_email         TEXT            NULL,

    -- For member-management events: the user the action is being performed
    -- on (invited, removed, role changed). target_email covers invites where
    -- no user row exists yet.
    target_user_id      UUID            NULL,
    target_email        TEXT            NULL,

    -- Event-specific payload (e.g. {"old_role":"MEMBER","new_role":"ADMIN"}).
    metadata            JSONB           NULL,

    ip_address          TEXT            NULL,
    user_agent          TEXT            NULL,
    ua_browser          TEXT            NULL,
    ua_os               TEXT            NULL,
    ua_device           TEXT            NULL,

    occurred_at         TIMESTAMPTZ     NOT NULL DEFAULT now()
);

-- Primary read path: "the security log for org X, newest first" with keyset pagination.
CREATE INDEX org_security_audit_log_org_occurred_idx
    ON org_security_audit_log (organization_id, occurred_at DESC, id DESC);

-- "Show me this actor's history" — used for member-profile drill-down.
CREATE INDEX org_security_audit_log_actor_idx
    ON org_security_audit_log (organization_id, actor_id, occurred_at DESC)
    WHERE actor_id IS NOT NULL;

-- "Show me everything involving this user" — invited then removed, etc.
CREATE INDEX org_security_audit_log_target_idx
    ON org_security_audit_log (organization_id, target_user_id, occurred_at DESC)
    WHERE target_user_id IS NOT NULL;
