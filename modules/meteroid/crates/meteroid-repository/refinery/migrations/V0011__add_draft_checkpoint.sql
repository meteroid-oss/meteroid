-- Create "checkpoint_draft_subscription" table
CREATE TABLE "checkpoint_draft_subscription" ("date" date NOT NULL, "created_at" timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP, "last_subscription_id" uuid NOT NULL, PRIMARY KEY ("date"));
-- Create index "checkpoint_draft_subscription_date_key" to table: "checkpoint_draft_subscription"
CREATE UNIQUE INDEX "checkpoint_draft_subscription_date_key" ON "checkpoint_draft_subscription" ("date");
