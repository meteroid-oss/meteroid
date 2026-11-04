-- A movement belongs to its subscription, not to an invoice: a usage-only subscription's creation,
-- a churn or an in-place change has no invoice of its own to point at.
ALTER TABLE bi_mrr_movement_log
    ALTER COLUMN invoice_id DROP NOT NULL,
    ADD COLUMN subscription_id uuid REFERENCES subscription ON UPDATE CASCADE ON DELETE RESTRICT,
    ADD COLUMN customer_id     uuid REFERENCES customer ON UPDATE CASCADE ON DELETE RESTRICT;

UPDATE bi_mrr_movement_log m
SET subscription_id = i.subscription_id,
    customer_id     = i.customer_id
FROM invoice i
WHERE i.id = m.invoice_id;

-- A movement whose invoice carries no subscription is still named by the event it booked.
UPDATE bi_mrr_movement_log m
SET subscription_id = e.subscription_id
FROM subscription_event e
WHERE e.bi_mrr_movement_log_id = m.id
  AND m.subscription_id IS NULL;

ALTER TABLE bi_mrr_movement_log
    ALTER COLUMN subscription_id SET NOT NULL,
    ALTER COLUMN customer_id SET NOT NULL;

CREATE INDEX bi_mrr_movement_log_subscription_idx ON bi_mrr_movement_log (subscription_id);
CREATE INDEX bi_mrr_movement_log_customer_idx ON bi_mrr_movement_log (customer_id);

-- A started subscription whose first period issued no invoice (usage only, or fees billed in
-- arrears) never booked its creation. Its `Created` event already holds the floored, net MRR:
-- book it as creation now does, where nothing else was booked since (no trial, not imported).
CREATE TEMP TABLE unbooked_creation ON COMMIT DROP AS
SELECT e.id AS event_id, uuidv7() AS log_id, e.mrr_delta, e.applies_to,
       s.id AS subscription_id, s.customer_id, s.tenant_id, s.plan_version_id, s.currency
FROM subscription_event e
JOIN subscription s ON s.id = e.subscription_id
WHERE e.event_type = 'CREATED'
  AND e.bi_mrr_movement_log_id IS NULL
  AND e.mrr_delta <> 0
  AND s.status = 'ACTIVE'
  AND s.trial_duration IS NULL
  AND s.imported_at IS NULL
  AND s.mrr_cents = 0
  AND NOT EXISTS (SELECT 1 FROM bi_mrr_movement_log m WHERE m.subscription_id = s.id);

INSERT INTO bi_mrr_movement_log (id, description, movement_type, net_mrr_change, currency, applies_to,
                                 plan_version_id, tenant_id, subscription_id, customer_id)
SELECT log_id, 'Subscription created', 'NEW_BUSINESS', mrr_delta, currency, applies_to,
       plan_version_id, tenant_id, subscription_id, customer_id
FROM unbooked_creation;

UPDATE subscription_event e
SET bi_mrr_movement_log_id = u.log_id
FROM unbooked_creation u
WHERE e.id = u.event_id;

UPDATE subscription s
SET mrr_cents = u.mrr_delta
FROM unbooked_creation u
WHERE s.id = u.subscription_id;
