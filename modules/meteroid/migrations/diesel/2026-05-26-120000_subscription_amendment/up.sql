-- Manual/sales-led subscription amendments.
-- 1) Temporal tracking on subscription_add_on so add-ons added/removed mid-cycle
--    bill for the correct window (mirrors subscription_component).
-- 2) APPLY_AMENDMENT scheduled-event type for end-of-period amendments.

ALTER TABLE subscription_add_on
  ADD COLUMN effective_from DATE NOT NULL DEFAULT CURRENT_DATE,
  ADD COLUMN effective_to DATE;

UPDATE subscription_add_on sao
SET effective_from = COALESCE(s.activated_at::date, s.billing_start_date, s.start_date)
FROM subscription s WHERE sao.subscription_id = s.id;

ALTER TYPE "ScheduledEventTypeEnum" ADD VALUE IF NOT EXISTS 'APPLY_AMENDMENT';

ALTER TABLE scheduled_event ADD COLUMN created_by_customer BOOLEAN NOT NULL DEFAULT false;


-- Lineage links a re-inserted (overridden) component / add-on back to the original
-- row it descends from. When a component is amended its row is closed (effective_to
-- set) and a NEW row inserted with a new id; without a lineage link the amendment
-- credit can no longer be matched to the originally-billed invoice line after the
-- first override. `lineage_id` always points at the lineage ROOT; NULL means the
-- row is its own root.

ALTER TABLE subscription_component
  ADD COLUMN lineage_id UUID REFERENCES subscription_component (id) ON DELETE SET NULL;

ALTER TABLE subscription_add_on
  ADD COLUMN lineage_id UUID REFERENCES subscription_add_on (id) ON DELETE SET NULL;

-- Backfill component lineage.
-- Add-ons are intentionally NOT backfilled: a subscription may hold several concurrent instances of the same add_on_id
WITH roots AS (
  SELECT subscription_id,
         price_component_id,
         (array_agg(id ORDER BY effective_from, id))[1] AS root_id
  FROM subscription_component
  WHERE price_component_id IS NOT NULL
  GROUP BY subscription_id, price_component_id
)
UPDATE subscription_component sc
SET lineage_id = roots.root_id
FROM roots
WHERE sc.subscription_id = roots.subscription_id
  AND sc.price_component_id = roots.price_component_id
  AND sc.id <> roots.root_id;



CREATE INDEX idx_sub_component_lineage
  ON subscription_component (lineage_id) WHERE lineage_id IS NOT NULL;

CREATE INDEX idx_sub_add_on_lineage
  ON subscription_add_on (lineage_id) WHERE lineage_id IS NOT NULL;
