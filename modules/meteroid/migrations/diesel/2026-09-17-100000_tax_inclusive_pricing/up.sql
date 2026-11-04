-- Tax-inclusive pricing (additive, non-breaking). FALSE everywhere keeps today's
-- behavior: amounts are net and tax is added on top.
--
-- The plan version says how its amounts read ("9.99 incl. VAT"), the same way it
-- says which currency they are in. A subscription snapshots it at creation, and an
-- invoice records the basis it was computed on so a refresh, a credit note or a
-- reissue never has to re-resolve it.
ALTER TABLE plan_version
  ADD COLUMN tax_inclusive BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE subscription
  ADD COLUMN tax_inclusive BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE invoice
  ADD COLUMN tax_inclusive BOOLEAN NOT NULL DEFAULT FALSE;
