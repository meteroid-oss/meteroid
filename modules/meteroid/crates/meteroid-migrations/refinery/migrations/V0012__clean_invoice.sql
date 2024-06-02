-- Modify "invoice" table
ALTER TABLE
    "invoice" DROP COLUMN "schedule_type",
    DROP COLUMN "grace_period_hours",
    ADD COLUMN "data_updated_at" timestamp(3),
ALTER COLUMN
    "last_issue_attempt_at" DROP DEFAULT;

-- Create "invoicing_config" table
CREATE TABLE "invoicing_config" (
    "id" uuid NOT NULL,
    "tenant_id" uuid NOT NULL,
    "grace_period_hours" integer NOT NULL,
    PRIMARY KEY ("id")
);

-- Drop enum type "InvoiceScheduleTypeEnum"
DROP TYPE "InvoiceScheduleTypeEnum";
