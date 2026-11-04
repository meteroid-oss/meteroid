-- When the provider last pushed new card details (number or expiry) for this method, e.g.
-- Stripe's card updater. An expired-card decline older than this no longer stops retries.
ALTER TABLE customer_payment_method ADD COLUMN card_refreshed_at TIMESTAMPTZ;
