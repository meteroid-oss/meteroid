-- Batch Job Engine: generic async batch processing with chunk-level parallelism

-- ============================================================================
-- Enum types
-- ============================================================================
CREATE TYPE "BatchJobTypeEnum" AS ENUM (
    'EVENT_CSV_IMPORT',
    'CUSTOMER_CSV_IMPORT',
    'SUBSCRIPTION_CSV_IMPORT',
    'SUBSCRIPTION_PLAN_MIGRATION'
);

CREATE TYPE "BatchJobStatusEnum" AS ENUM (
    'PENDING',
    'CHUNKING',
    'PROCESSING',
    'COMPLETED',
    'COMPLETED_WITH_ERRORS',
    'FAILED',
    'CANCELLED'
);

CREATE TYPE "BatchJobChunkStatusEnum" AS ENUM (
    'PENDING',
    'PROCESSING',
    'COMPLETED',
    'FAILED',
    'SKIPPED'
);

-- ============================================================================
-- batch_job: top-level job record
-- ============================================================================
CREATE TABLE batch_job (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE RESTRICT,
    job_type "BatchJobTypeEnum" NOT NULL,
    status "BatchJobStatusEnum" NOT NULL DEFAULT 'PENDING',
    input_source_key TEXT,
    input_params JSONB,
    total_items INT4,
    processed_items INT4 NOT NULL DEFAULT 0,
    failed_items INT4 NOT NULL DEFAULT 0,
    file_hash TEXT,
    locked_at TIMESTAMP,
    created_by UUID NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    error_message TEXT,
    error_output_key TEXT,
    input_file_name TEXT
);

CREATE INDEX idx_batch_job_tenant ON batch_job(tenant_id);
CREATE INDEX idx_batch_job_status ON batch_job(status);
CREATE INDEX idx_batch_job_pending ON batch_job(created_at) WHERE status = 'PENDING';
CREATE INDEX idx_batch_job_dedup ON batch_job(tenant_id, job_type, file_hash) WHERE file_hash IS NOT NULL;

-- ============================================================================
-- batch_job_chunk: independently retryable unit within a job
-- ============================================================================
CREATE TABLE batch_job_chunk (
    id UUID PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES batch_job(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE RESTRICT,
    chunk_index INT4 NOT NULL,
    status "BatchJobChunkStatusEnum" NOT NULL DEFAULT 'PENDING',
    item_offset INT4 NOT NULL,
    item_count INT4 NOT NULL,
    processed_count INT4 NOT NULL DEFAULT 0,
    failed_count INT4 NOT NULL DEFAULT 0,
    retry_count INT4 NOT NULL DEFAULT 0,
    max_retries INT4 NOT NULL DEFAULT 3,
    locked_at TIMESTAMP,
    retry_after TIMESTAMP,
    events JSONB NOT NULL DEFAULT '[]'::JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_batch_job_chunk_job ON batch_job_chunk(job_id);
CREATE INDEX idx_batch_job_chunk_pending ON batch_job_chunk(job_id, chunk_index)
    WHERE status = 'PENDING';
CREATE INDEX idx_batch_job_chunk_stuck ON batch_job_chunk(locked_at)
    WHERE status = 'PROCESSING';

-- ============================================================================
-- batch_job_item_failure: per-item failure tracking
-- ============================================================================
CREATE TABLE batch_job_item_failure (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    chunk_id UUID NOT NULL REFERENCES batch_job_chunk(id) ON DELETE CASCADE,
    job_id UUID NOT NULL REFERENCES batch_job(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE RESTRICT,
    item_index INT4 NOT NULL,
    item_identifier TEXT,
    reason TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_batch_job_item_failure_job ON batch_job_item_failure(job_id);
CREATE INDEX idx_batch_job_item_failure_chunk ON batch_job_item_failure(chunk_id);

-- ============================================================================
-- batch_job_entity: tracks entities created/updated by batch jobs
-- ============================================================================
CREATE TABLE batch_job_entity (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    batch_job_id UUID NOT NULL REFERENCES batch_job(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE RESTRICT,
    entity_type TEXT NOT NULL,
    entity_id UUID NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_batch_job_entity_job ON batch_job_entity(batch_job_id);
CREATE INDEX idx_batch_job_entity_entity ON batch_job_entity(entity_type, entity_id);
