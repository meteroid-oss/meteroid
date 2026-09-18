-- Provider id of the underlying instrument (Stripe `fingerprint`, Mollie `cardFingerprint`,
-- hashed IBAN), used to merge re-added cards/mandates into one row. NULLs never collide.
ALTER TABLE customer_payment_method ADD COLUMN fingerprint TEXT;

CREATE UNIQUE INDEX customer_payment_method_active_fingerprint_uidx
    ON customer_payment_method (connection_id, payment_method_type, fingerprint)
    WHERE archived_at IS NULL;
