-- The tax rate embedded in a tax-inclusive subscription's quoted prices, resolved once at
-- creation the way an invoice resolves it: the invoicing entity's tax engine, the customer,
-- and one line per component/add-on carrying its own product and category configuration.
-- Components that resolve to different rates are blended by their quoted amounts, so the
-- one stored figure is what the subscription as a whole carries per unit of net revenue.
--
-- MRR is read off the subscription's fee amounts, and on a tax-inclusive plan those are
-- the quoted (tax-included) figures — so the MRR would carry the tax with them. This is
-- the divisor that takes it back out, snapshotted alongside `tax_inclusive` so the figure
-- never depends on re-resolving tax at read time.
--
-- NULL means "nothing to take out": a tax-exclusive subscription, or a tax-inclusive one
-- whose customer bears no tax (exempt, reverse charge, export, no tax engine).
ALTER TABLE subscription
  ADD COLUMN tax_inclusive_rate NUMERIC NULL;
