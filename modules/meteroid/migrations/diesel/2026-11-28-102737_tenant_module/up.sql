-- The modules a tenant turned on, by module id (e.g. `ai`): a module listed here is active for
-- the tenant on top of what the organization's license grants. Absent means off.
CREATE TABLE tenant_module (
    tenant_id  UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    module     TEXT        NOT NULL CHECK (module ~ '^[a-z][a-z0-9-]{0,63}$'),
    enabled_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, module)
);
