-- Extend custom (user-defined) properties to quotes.
-- See 2026-07-23-120000_credit_note_custom_properties for the previous rollout.

ALTER TYPE "CustomPropertyEntityTypeEnum" ADD VALUE IF NOT EXISTS 'QUOTE';

-- Per-entity value bag. Keyed by definition `key`; empty object when unset.
ALTER TABLE "quote" ADD COLUMN "custom_properties" JSONB NOT NULL DEFAULT '{}'::jsonb;

-- GIN index so list filters can match on property values.
CREATE INDEX "quote_custom_properties_idx" ON "quote" USING GIN ("custom_properties");
