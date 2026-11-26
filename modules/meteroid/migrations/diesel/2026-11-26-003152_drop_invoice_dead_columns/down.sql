ALTER TABLE invoice
    ADD COLUMN issued_at TIMESTAMPTZ,
    ADD COLUMN subtotal_recurring BIGINT NOT NULL DEFAULT 0;
ALTER TABLE invoice ALTER COLUMN subtotal_recurring DROP DEFAULT;
