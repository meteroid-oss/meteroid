-- InvoicePaymentStatus 'CARRIED' cannot be removed (Postgres has no DROP VALUE).
DROP TABLE IF EXISTS invoice_carry_forward;
ALTER TABLE invoicing_entity DROP COLUMN IF EXISTS min_charge_amount;
ALTER TABLE invoice DROP COLUMN IF EXISTS carried_balance;
