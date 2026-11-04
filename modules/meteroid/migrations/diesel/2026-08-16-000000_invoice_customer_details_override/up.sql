-- A draft is not an issued document: its customer identity tracks the live customer record and is
-- only frozen into customer_details at finalization. Stamped when an operator edits the billing
-- identity on a specific draft (a subsidiary, a "bill to" department, a VAT id corrected on paper
-- before the CRM catches up), which opts that invoice out of tracking.
--
-- Without this marker a deliberate inline edit and a merely stale snapshot are indistinguishable,
-- since both live in customer_details.
ALTER TABLE invoice
    ADD COLUMN customer_details_overridden_at TIMESTAMP;
