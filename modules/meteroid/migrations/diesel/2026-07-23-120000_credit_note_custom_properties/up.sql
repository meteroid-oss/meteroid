-- Extend custom (user-defined) properties to credit notes.
-- See 2026-07-15-120000_custom_properties for the customer/subscription/invoice rollout.

ALTER TYPE "CustomPropertyEntityTypeEnum" ADD VALUE IF NOT EXISTS 'CREDIT_NOTE';

-- Per-entity value bag. Keyed by definition `key`; empty object when unset.
ALTER TABLE "credit_note" ADD COLUMN "custom_properties" JSONB NOT NULL DEFAULT '{}'::jsonb;

-- GIN index so analytics/workflows can filter credit notes by property values.
CREATE INDEX "credit_note_custom_properties_idx" ON "credit_note" USING GIN ("custom_properties");
