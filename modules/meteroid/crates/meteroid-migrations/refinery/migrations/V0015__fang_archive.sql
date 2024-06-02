-- replicating same schema as fang_tasks table
-- but ONLY with index for further removal - archived_at
CREATE TABLE fang_tasks_archive (
    id uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    metadata jsonb NOT NULL,
    error_message TEXT,
    state fang_task_state DEFAULT 'new' NOT NULL,
    task_type VARCHAR DEFAULT 'common' NOT NULL,
    uniq_hash CHAR(64),
    retries INTEGER DEFAULT 0 NOT NULL,
    scheduled_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    -- additional column
    archived_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX fang_tasks_archive_archived_at_index ON fang_tasks_archive(archived_at);
