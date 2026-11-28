-- Workflows (ADR-0016): a workflow's draft and its frozen versions, the runs a version starts and
-- each step a run executes. Rows are owned by the workflows module; the matcher reads every
-- outbox event from the `workflow_engine` queue.
ALTER TYPE "ActorTypeEnum" ADD VALUE IF NOT EXISTS 'WORKFLOW';

CREATE TYPE "WorkflowTriggerTypeEnum" AS ENUM ('EVENT', 'SCHEDULE', 'MANUAL');
CREATE TYPE "WorkflowRunStatusEnum" AS ENUM ('PENDING', 'RUNNING', 'WAITING', 'SUCCEEDED', 'FAILED', 'CANCELLED');
CREATE TYPE "WorkflowStepStatusEnum" AS ENUM ('RUNNING', 'SUCCEEDED', 'FAILED', 'SKIPPED');

CREATE TABLE workflow (
    id                UUID PRIMARY KEY,
    tenant_id         UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    description       TEXT,
    draft             JSONB NOT NULL,
    active_version_id UUID,
    -- Copied from the active version, for the matcher and the scheduler.
    trigger_type      "WorkflowTriggerTypeEnum",
    trigger_event     TEXT,
    next_fire_at      TIMESTAMPTZ,
    created_by        UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at       TIMESTAMPTZ
);

CREATE INDEX workflow_tenant_idx ON workflow (tenant_id, created_at DESC) WHERE archived_at IS NULL;
CREATE INDEX workflow_event_idx ON workflow (tenant_id, trigger_event)
    WHERE active_version_id IS NOT NULL AND trigger_type = 'EVENT';
CREATE INDEX workflow_schedule_due_idx ON workflow (next_fire_at)
    WHERE active_version_id IS NOT NULL AND trigger_type = 'SCHEDULE';

CREATE TABLE workflow_version (
    id          UUID PRIMARY KEY,
    tenant_id   UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    workflow_id UUID NOT NULL REFERENCES workflow (id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    definition  JSONB NOT NULL,
    -- The API v2 version its actions are sent at, fixed when activated.
    api_version TEXT NOT NULL,
    note        TEXT,
    created_by  UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (workflow_id, version)
);

ALTER TABLE workflow
    ADD CONSTRAINT workflow_active_version_fkey
    FOREIGN KEY (active_version_id) REFERENCES workflow_version (id);

CREATE TABLE workflow_run (
    id               UUID PRIMARY KEY,
    tenant_id        UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    workflow_id      UUID NOT NULL REFERENCES workflow (id) ON DELETE CASCADE,
    version_id       UUID NOT NULL REFERENCES workflow_version (id) ON DELETE CASCADE,
    trigger_type     "WorkflowTriggerTypeEnum" NOT NULL,
    trigger_event_id UUID,
    idempotency_key  TEXT,
    status           "WorkflowRunStatusEnum" NOT NULL,
    -- The path to the next node to execute, loop indexes included.
    position         JSONB NOT NULL DEFAULT '[]'::jsonb,
    context          JSONB NOT NULL,
    -- The runs whose actions caused the trigger event, oldest first (recursion guard).
    origin           JSONB NOT NULL DEFAULT '[]'::jsonb,
    next_wake_at     TIMESTAMPTZ,
    locked_until     TIMESTAMPTZ,
    error_code       TEXT,
    error_message    TEXT,
    failed_node_id   TEXT,
    created_by       UUID,
    started_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at      TIMESTAMPTZ
);

CREATE UNIQUE INDEX workflow_run_trigger_event_idx ON workflow_run (workflow_id, trigger_event_id)
    WHERE trigger_event_id IS NOT NULL;
CREATE UNIQUE INDEX workflow_run_idempotency_idx ON workflow_run (workflow_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX workflow_run_due_idx ON workflow_run (next_wake_at)
    WHERE status IN ('PENDING', 'WAITING');
CREATE INDEX workflow_run_lease_idx ON workflow_run (locked_until) WHERE status = 'RUNNING';
CREATE INDEX workflow_run_tenant_idx ON workflow_run (tenant_id, started_at DESC, id DESC);
CREATE INDEX workflow_run_workflow_idx ON workflow_run (workflow_id, started_at DESC, id DESC);
-- Daily counts of a tenant's runs, read from the index alone.
CREATE INDEX workflow_run_stats_idx ON workflow_run (tenant_id, started_at) INCLUDE (workflow_id, status);
CREATE INDEX workflow_run_finished_idx ON workflow_run (finished_at) WHERE finished_at IS NOT NULL;

CREATE TABLE workflow_step_run (
    id            UUID PRIMARY KEY,
    tenant_id     UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    run_id        UUID NOT NULL REFERENCES workflow_run (id) ON DELETE CASCADE,
    node_id       TEXT NOT NULL,
    -- Loop indexes from the outermost loop, dot-separated; empty outside any loop.
    iteration     TEXT NOT NULL DEFAULT '',
    kind          TEXT NOT NULL,
    status        "WorkflowStepStatusEnum" NOT NULL,
    attempts      INTEGER NOT NULL DEFAULT 0,
    input         JSONB,
    output        JSONB,
    error_code    TEXT,
    error_message TEXT,
    request_id    TEXT,
    started_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at   TIMESTAMPTZ,
    UNIQUE (run_id, node_id, iteration)
);

-- The latest events of each type, rendered as a trigger sees them: what a draft is tried on.
CREATE TABLE workflow_event_sample (
    id         UUID PRIMARY KEY,
    tenant_id  UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    envelope   JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX workflow_event_sample_idx ON workflow_event_sample (tenant_id, event_type, created_at DESC);
CREATE INDEX workflow_event_sample_created_idx ON workflow_event_sample (created_at);

SELECT pgmq.create('workflow_engine');
SELECT pgmq.bind_topic('#', 'workflow_engine');
