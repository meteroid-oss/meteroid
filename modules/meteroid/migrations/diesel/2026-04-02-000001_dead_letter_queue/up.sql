CREATE TYPE "DeadLetterStatusEnum" AS ENUM ('PENDING', 'REQUEUED', 'DISCARDED');

CREATE TABLE dead_letter_message (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    queue TEXT NOT NULL,
    pgmq_msg_id BIGINT NOT NULL,
    message JSONB,
    headers JSONB,
    read_ct INT NOT NULL,
    enqueued_at TIMESTAMPTZ NOT NULL,
    dead_lettered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    status "DeadLetterStatusEnum" NOT NULL DEFAULT 'PENDING',
    resolved_at TIMESTAMPTZ,
    resolved_by UUID,
    requeued_pgmq_msg_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_dlq_queue_status ON dead_letter_message(queue, status);
CREATE INDEX idx_dlq_pending ON dead_letter_message(dead_lettered_at) WHERE status = 'PENDING';

-- Tracks last alert time per queue for distributed-safe webhook debouncing
CREATE TABLE dead_letter_alert_state (
    queue TEXT PRIMARY KEY,
    last_alerted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
