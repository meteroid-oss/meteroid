DROP VIEW IF EXISTS current_billing_period;

ALTER TABLE "subscription"
  DROP COLUMN "status";
DROP TYPE IF EXISTS "SubscriptionStatusEnum";

ALTER TABLE "subscription"
  ADD COLUMN "canceled_at" TIMESTAMP(3);
ALTER TABLE "subscription"
  ADD COLUMN "cancellation_reason" TEXT;
