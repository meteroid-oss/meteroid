-- Debts carried in from earlier invoices less the debt carried to a later one, owed on top of the
-- total: amount_due = total + carried_balance - applied_credits - paid. Each carried debt is an
-- invoice_carry_forward row, from which the two parts are read.
ALTER TABLE invoice
    ADD COLUMN carried_balance BIGINT NOT NULL DEFAULT 0;

-- An invoice whose amount due is owed on the next invoice: not collected, not paid until that
-- invoice is. Not used in this migration, as a value added in a transaction cannot be.
ALTER TYPE "InvoicePaymentStatus" ADD VALUE IF NOT EXISTS 'CARRIED';

-- A floor above the payment providers' minimum, in the entity's currency; NULL adds none.
ALTER TABLE invoicing_entity
    ADD COLUMN min_charge_amount BIGINT CHECK (min_charge_amount >= 0);

CREATE TABLE invoice_carry_forward
(
    id                  UUID PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    customer_id         UUID        NOT NULL REFERENCES customer ON DELETE CASCADE,
    invoicing_entity_id UUID        NOT NULL REFERENCES invoicing_entity ON DELETE CASCADE,
    currency            TEXT        NOT NULL,
    -- 0 once credit notes cancelled the whole debt.
    amount              BIGINT      NOT NULL CHECK (amount >= 0),
    source_invoice_id   UUID        NOT NULL UNIQUE REFERENCES invoice ON DELETE CASCADE,
    applied_invoice_id  UUID REFERENCES invoice ON DELETE SET NULL,
    applied_at          TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX invoice_carry_forward_pending_idx
    ON invoice_carry_forward (tenant_id, customer_id, invoicing_entity_id, currency)
    WHERE applied_invoice_id IS NULL;

CREATE INDEX invoice_carry_forward_applied_idx
    ON invoice_carry_forward (applied_invoice_id)
    WHERE applied_invoice_id IS NOT NULL;
