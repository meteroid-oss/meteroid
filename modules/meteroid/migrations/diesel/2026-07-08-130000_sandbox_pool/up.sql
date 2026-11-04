CREATE TYPE "SandboxPoolStatusEnum" AS ENUM ('WARMING', 'READY', 'CLAIMED');

-- Pre-warmed sandbox tenants owned by the system org, waiting to be claimed
-- (re-parented) by a new organization on signup. One row per pooled tenant.
CREATE TABLE sandbox_pool_tenant (
    tenant_id  UUID PRIMARY KEY REFERENCES tenant (id) ON DELETE CASCADE,
    status     "SandboxPoolStatusEnum" NOT NULL DEFAULT 'WARMING',
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    ready_at   TIMESTAMP,
    claimed_at TIMESTAMP
);

CREATE INDEX idx_sandbox_pool_tenant_status ON sandbox_pool_tenant (status);
