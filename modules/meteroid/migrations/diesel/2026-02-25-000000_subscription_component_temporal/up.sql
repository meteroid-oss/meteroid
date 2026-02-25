ALTER TABLE subscription_component
  ADD COLUMN effective_from DATE NOT NULL DEFAULT CURRENT_DATE,
  ADD COLUMN effective_to DATE;

UPDATE subscription_component sc
SET effective_from = COALESCE(s.activated_at::date, s.billing_start_date, s.start_date)
FROM subscription s WHERE sc.subscription_id = s.id;

CREATE INDEX idx_sub_component_active
  ON subscription_component(subscription_id) WHERE effective_to IS NULL;

CREATE INDEX idx_sub_component_period_overlap
  ON subscription_component(subscription_id, effective_from, effective_to);
