ALTER TABLE payment_transaction DROP COLUMN amount_refunded;

-- PaymentStatusEnum 'REFUNDED' cannot be removed (Postgres has no DROP VALUE);
-- rolling it back requires recreating the type manually.
