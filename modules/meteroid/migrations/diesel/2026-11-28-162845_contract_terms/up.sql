-- A quote's contract term (ADR-0022 §2): signed with the quote, written as its subscription's term
-- on conversion, counted in months from the billing start.
CREATE TABLE quote_term (
    quote_id       UUID PRIMARY KEY REFERENCES quote (id) ON DELETE CASCADE,
    tenant_id      UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    term_months    INTEGER     NOT NULL CHECK (term_months BETWEEN 1 AND 120),
    auto_renew     BOOLEAN     NOT NULL,
    renewal_months INTEGER     NOT NULL CHECK (renewal_months BETWEEN 1 AND 120),
    notice_days    INTEGER     NOT NULL DEFAULT 0 CHECK (notice_days >= 0),
    uplift_percent NUMERIC CHECK (uplift_percent >= 0),
    updated_at     TIMESTAMP   NOT NULL
);

-- How a subscription was sold (ADR-0022 §4); null when nothing proves it. Backfilled only from
-- proof: its quote, its import, its self-serve checkout.
ALTER TABLE subscription
    ADD COLUMN IF NOT EXISTS sold_via TEXT,
    ADD CONSTRAINT subscription_sold_via_check
        CHECK (sold_via IN ('SELF_SERVE', 'QUOTE', 'APP', 'API', 'IMPORT'));

UPDATE subscription SET sold_via = 'QUOTE' WHERE quote_id IS NOT NULL;
UPDATE subscription SET sold_via = 'IMPORT' WHERE sold_via IS NULL AND imported_at IS NOT NULL;
UPDATE subscription s SET sold_via = 'SELF_SERVE'
WHERE s.sold_via IS NULL
  AND EXISTS (SELECT 1 FROM checkout_session cs
              WHERE cs.subscription_id = s.id AND cs.checkout_type = 'SELF_SERVE');
