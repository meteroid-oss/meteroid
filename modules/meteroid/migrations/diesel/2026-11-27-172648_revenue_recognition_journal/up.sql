-- Revenue recognition journals (ADR-0014 §2, §5): the months an entity closed, and the entries a
-- close posts. Posted entries never change, and every entry balances in the functional currency.
CREATE TYPE "RevrecPoolEnum" AS ENUM ('CONTRACT', 'CREDIT', 'RECEIVABLE', 'MANUAL');
CREATE TYPE "JournalEntryKindEnum" AS ENUM ('OPENING', 'CLOSE', 'MANUAL');
CREATE TYPE "JournalEntryStatusEnum" AS ENUM ('DRAFT', 'POSTED');

CREATE TABLE revrec_period (
    invoicing_entity_id UUID        NOT NULL,
    tenant_id           UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    month               DATE        NOT NULL CHECK (extract(day FROM month) = 1),
    closed_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_by           UUID,
    PRIMARY KEY (invoicing_entity_id, month),
    CONSTRAINT revrec_period_entity_tenant_fkey FOREIGN KEY (invoicing_entity_id, tenant_id)
        REFERENCES invoicing_entity (id, tenant_id) ON DELETE CASCADE
);

CREATE TABLE journal_entry (
    id                  UUID PRIMARY KEY,
    tenant_id           UUID                     NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    invoicing_entity_id UUID                     NOT NULL,
    period              DATE                     NOT NULL CHECK (extract(day FROM period) = 1),
    kind                "JournalEntryKindEnum"   NOT NULL,
    status              "JournalEntryStatusEnum" NOT NULL DEFAULT 'DRAFT',
    posting_date        DATE                     NOT NULL,
    functional_currency TEXT                     NOT NULL,
    memo                TEXT                     NOT NULL DEFAULT '' CHECK (length(memo) <= 500),
    created_by          UUID,
    approved_by         UUID,
    created_at          TIMESTAMPTZ              NOT NULL DEFAULT now(),
    posted_at           TIMESTAMPTZ,
    CONSTRAINT journal_entry_posted_at CHECK ((status = 'POSTED') = (posted_at IS NOT NULL)),
    -- A manual entry posts once someone other than its author approved it.
    CONSTRAINT journal_entry_four_eyes CHECK (
        kind <> 'MANUAL' OR status <> 'POSTED'
        OR (approved_by IS NOT NULL AND created_by IS NOT NULL AND approved_by <> created_by)
    ),
    CONSTRAINT journal_entry_manual_memo CHECK (kind <> 'MANUAL' OR length(btrim(memo)) > 0),
    CONSTRAINT journal_entry_entity_tenant_fkey FOREIGN KEY (invoicing_entity_id, tenant_id)
        REFERENCES invoicing_entity (id, tenant_id) ON DELETE CASCADE
);
CREATE INDEX journal_entry_entity_idx ON journal_entry (invoicing_entity_id, period);
CREATE UNIQUE INDEX journal_entry_one_draft ON journal_entry (invoicing_entity_id, period, kind)
    WHERE status = 'DRAFT' AND kind <> 'MANUAL';

-- A line keeps the position it moved (customer, currency, pool, contract or invoice, product) so the
-- next close knows what was posted; a manual line has none. Customers are not referenced: the books
-- outlive them.
CREATE TABLE journal_line (
    id                  UUID PRIMARY KEY,
    entry_id            UUID                    NOT NULL REFERENCES journal_entry (id) ON DELETE CASCADE,
    invoicing_entity_id UUID                    NOT NULL,
    ledger_account_id   UUID                    NOT NULL,
    role                "LedgerAccountRoleEnum",
    customer_id         UUID,
    currency            TEXT                    NOT NULL,
    pool                "RevrecPoolEnum"        NOT NULL,
    contract_id         UUID,
    product_id          UUID,
    amount              BIGINT                  NOT NULL,
    functional_amount   BIGINT                  NOT NULL,
    CONSTRAINT journal_line_account_fkey FOREIGN KEY (ledger_account_id, invoicing_entity_id)
        REFERENCES ledger_account (id, invoicing_entity_id) ON DELETE RESTRICT,
    CONSTRAINT journal_line_pool CHECK (
        (pool <> 'CONTRACT' OR contract_id IS NOT NULL) AND (pool <> 'CREDIT' OR contract_id IS NULL)
        AND ((pool = 'MANUAL') = (customer_id IS NULL)) AND (pool = 'MANUAL' OR role IS NOT NULL)
    )
);
CREATE INDEX journal_line_entry_idx ON journal_line (entry_id);
CREATE INDEX journal_line_position_idx ON journal_line (invoicing_entity_id, customer_id);

CREATE FUNCTION journal_entry_balanced() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    entry UUID := coalesce(NEW.entry_id, OLD.entry_id);
    total NUMERIC;
BEGIN
    SELECT coalesce(sum(functional_amount), 0) INTO total FROM journal_line WHERE entry_id = entry;
    IF total <> 0 THEN
        RAISE EXCEPTION 'journal entry % does not balance (%)', entry, total USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER journal_line_balanced
    AFTER INSERT OR UPDATE OR DELETE ON journal_line
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION journal_entry_balanced();

-- Deleting the tenant or the entity still cascades (a trigger deeper than the statement).
CREATE FUNCTION journal_posted_is_final() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    IF TG_TABLE_NAME = 'journal_entry' THEN
        IF OLD.status = 'POSTED' THEN
            RAISE EXCEPTION 'posted journal entry % cannot change', OLD.id USING ERRCODE = 'check_violation';
        END IF;
    ELSIF EXISTS (SELECT 1 FROM journal_entry
                  WHERE id = coalesce(OLD.entry_id, NEW.entry_id) AND status = 'POSTED') THEN
        RAISE EXCEPTION 'lines of a posted journal entry cannot change' USING ERRCODE = 'check_violation';
    END IF;
    RETURN coalesce(NEW, OLD);
END $$;
CREATE TRIGGER journal_entry_final BEFORE UPDATE OR DELETE ON journal_entry
    FOR EACH ROW EXECUTE FUNCTION journal_posted_is_final();
CREATE TRIGGER journal_line_final BEFORE INSERT OR UPDATE OR DELETE ON journal_line
    FOR EACH ROW EXECUTE FUNCTION journal_posted_is_final();
