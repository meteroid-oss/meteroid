-- Contract terms (ADR-0014 §4): the revisions of a committed subscription's fixed consideration, per
-- product, each written by the close that first saw it. A modification adds a revision, recognized
-- prospectively from its date on top of what the earlier ones recognized before it (`carried`).
CREATE TABLE revrec_contract (
    id                  UUID PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    invoicing_entity_id UUID        NOT NULL,
    subscription_id     UUID        NOT NULL,
    product_id          UUID,
    currency            TEXT        NOT NULL,
    effective_from      DATE        NOT NULL,
    term_end            DATE        NOT NULL,
    scheduled           BIGINT      NOT NULL,
    carried             BIGINT      NOT NULL,
    period              DATE        NOT NULL CHECK (extract(day FROM period) = 1),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT revrec_contract_entity_tenant_fkey FOREIGN KEY (invoicing_entity_id, tenant_id)
        REFERENCES invoicing_entity (id, tenant_id) ON DELETE CASCADE
);
CREATE INDEX revrec_contract_subscription_idx ON revrec_contract (invoicing_entity_id, subscription_id);

-- What the books' revenue differs from Intelligence's by through the month: the contracts' straight-lining.
ALTER TABLE revrec_period ADD COLUMN contract_adjustment BIGINT NOT NULL DEFAULT 0;
