-- Add imported_at column to track when subscriptions were migrated with skip_past_invoices
ALTER TABLE subscription ADD COLUMN imported_at TIMESTAMP;
CREATE INDEX idx_subscription_imported_at ON subscription(imported_at) WHERE imported_at IS NOT NULL;
