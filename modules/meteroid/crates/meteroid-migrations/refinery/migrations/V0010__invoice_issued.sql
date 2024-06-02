-- Modify "invoice" table
ALTER TABLE "invoice"
  ADD COLUMN "issued"                boolean        NOT NULL DEFAULT false,
  ADD COLUMN "issue_attempts"        integer        NOT NULL DEFAULT 0,
  ADD COLUMN "last_issue_attempt_at" timestamptz(3) NULL     DEFAULT NULL,
  ADD COLUMN "last_issue_error"      text           NULL DEFAULT NULL;
