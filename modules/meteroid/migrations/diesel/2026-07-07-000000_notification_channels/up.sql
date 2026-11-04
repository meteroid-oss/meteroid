CREATE TYPE "ChannelTypeEnum" AS ENUM('SLACK', 'EMAIL', 'WEBHOOK', 'DISCORD');

-- Tenant-wide, reusable delivery destinations. Configured once, then referenced by
-- alert definitions (signals and monitors) via a {"type":"channel","id":...} entry in
-- their channels JSON, so a Slack/email/webhook is set up in one place and reused.
CREATE TABLE notification_channel (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    channel_type "ChannelTypeEnum" NOT NULL,
    config JSONB NOT NULL,
    disabled BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP,
    archived_at TIMESTAMP
);

CREATE INDEX idx_notification_channel_tenant ON notification_channel(tenant_id);

CREATE UNIQUE INDEX idx_notification_channel_tenant_name ON notification_channel (tenant_id, name) WHERE archived_at IS NULL;
