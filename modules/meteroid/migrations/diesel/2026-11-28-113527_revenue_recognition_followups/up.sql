-- The contract adjustment of each contract product through a closed month, in the transaction and
-- the functional currency: a later close converts only what it adds (ADR-0014 §4 Currency).
CREATE TABLE revrec_contract_adjustment (
    id                  UUID PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    invoicing_entity_id UUID        NOT NULL,
    subscription_id     UUID        NOT NULL,
    product_id          UUID,
    through             DATE        NOT NULL,
    amount              BIGINT      NOT NULL,
    functional_amount   BIGINT      NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT revrec_contract_adjustment_entity_tenant_fkey FOREIGN KEY (invoicing_entity_id, tenant_id)
        REFERENCES invoicing_entity (id, tenant_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX revrec_contract_adjustment_key
    ON revrec_contract_adjustment (invoicing_entity_id, subscription_id, product_id, through) NULLS NOT DISTINCT;

-- When an export first carried a posted entry (ADR-0014 §3): the one thing a posted entry can
-- still take, once.
ALTER TABLE journal_entry ADD COLUMN exported_at TIMESTAMPTZ;

CREATE OR REPLACE FUNCTION journal_posted_is_final() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    IF TG_TABLE_NAME = 'journal_entry' THEN
        IF TG_OP = 'UPDATE' AND OLD.exported_at IS NULL AND NEW.exported_at IS NOT NULL
           AND to_jsonb(NEW) - 'exported_at' = to_jsonb(OLD) - 'exported_at' THEN
            RETURN NEW;
        END IF;
        IF OLD.status = 'POSTED' THEN
            RAISE EXCEPTION 'posted journal entry % cannot change', OLD.id USING ERRCODE = 'check_violation';
        END IF;
    ELSIF EXISTS (SELECT 1 FROM journal_entry
                  WHERE id = coalesce(OLD.entry_id, NEW.entry_id) AND status = 'POSTED') THEN
        RAISE EXCEPTION 'lines of a posted journal entry cannot change' USING ERRCODE = 'check_violation';
    END IF;
    RETURN coalesce(NEW, OLD);
END $$;

-- Deferrals only (ADR-0014 §3): invoices, cash and tax reach the ledger through a document export
-- (DATEV); the journals move what is billed and not earned between revenue and deferred revenue.
ALTER TYPE "RevrecPostingScopeEnum" ADD VALUE IF NOT EXISTS 'DEFERRALS';
