-- Serves the draft-refresh sweep, which pages through refreshable drafts ordered by id.
--
-- Without it the planner walks invoice_pkey and filters: on 500k invoices with 800 drafts,
-- one 101-row page discarded 67,720 rows (68k buffers, 87ms), and the cost grows with the
-- table forever since the sweep runs hourly. With this index the same page is a bitmap scan
-- over the drafts alone — 0.6ms, and independent of how many finalized invoices exist.
--
-- Partial on the three stable predicates that make a draft a candidate. The sweep's remaining
-- filters (invoice_type, subscription_id, the period bounds, staleness, payments in flight)
-- stay as a recheck: they change over a row's life or need a subquery, so they cannot live in
-- an index predicate. `id` is the indexed column because the cursor pagination orders by it.
--
-- Non-concurrent, matching the convention of the billing_period_start migration: cheap while
-- the table is small. On a large table, split this into a non-transactional migration and use
-- CREATE INDEX CONCURRENTLY instead.
CREATE INDEX idx_invoice_refreshable_drafts
    ON invoice (id)
    WHERE status = 'DRAFT'::"InvoiceStatusEnum"
      AND consolidated_into_invoice_id IS NULL
      AND manually_edited_at IS NULL;
