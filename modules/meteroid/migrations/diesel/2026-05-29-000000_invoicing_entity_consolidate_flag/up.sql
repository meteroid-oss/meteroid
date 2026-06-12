-- Opt-in: when true, this invoicing entity merges same-day eligible recurring drafts for a
-- customer into a single consolidated invoice. Off by default.
ALTER TABLE invoicing_entity
    ADD COLUMN consolidate_recurring_invoices BOOLEAN NOT NULL DEFAULT FALSE;
