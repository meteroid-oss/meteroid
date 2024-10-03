ALTER TABLE "invoice"
  ADD COLUMN "xml_document_id" TEXT;
ALTER TABLE "invoice"
  ADD COLUMN "pdf_document_id" TEXT;

create type "OutboxStatus" as enum ('PENDING', 'PROCESSING', 'COMPLETED', 'FAILED');

CREATE TABLE "outbox"
(
  "id"                      UUID                                             NOT NULL PRIMARY KEY,
  "event_type"              TEXT                                             NOT NULL,
  "tenant_id"               UUID                                             NOT NULL,
  "resource_id"             UUID                                             NOT NULL,
  "status"                  "OutboxStatus" default 'PENDING'::"OutboxStatus" NOT NULL,
  "payload"                 JSONB,
  "created_at"              timestamp(3)   default CURRENT_TIMESTAMP         NOT NULL,
  "processing_started_at"   timestamp(3),
  "processing_completed_at" timestamp(3),
  "processing_attempts"     INT4           default 0                         NOT NULL,
  "error"                   TEXT
);

CREATE INDEX idx_outbox_status ON outbox (status);
CREATE INDEX idx_outbox_event_type ON outbox (event_type);
