-- DATEV export (EE module): each invoicing entity's DATEV client and the account number each of its
-- customers books on there. Numbers are never reused: a tax adviser's books already carry them.
CREATE TABLE datev_settings (
    invoicing_entity_id UUID PRIMARY KEY REFERENCES invoicing_entity (id) ON DELETE CASCADE,
    tenant_id           UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    config              JSONB       NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT datev_settings_entity_tenant_fkey FOREIGN KEY (invoicing_entity_id, tenant_id)
        REFERENCES invoicing_entity (id, tenant_id) ON DELETE CASCADE
);
CREATE INDEX datev_settings_tenant_idx ON datev_settings (tenant_id);

CREATE TABLE datev_debtor (
    invoicing_entity_id UUID        NOT NULL REFERENCES invoicing_entity (id) ON DELETE CASCADE,
    customer_id         UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    tenant_id           UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    account             BIGINT      NOT NULL CHECK (account > 0),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (invoicing_entity_id, customer_id),
    CONSTRAINT datev_debtor_account_key UNIQUE (invoicing_entity_id, account),
    CONSTRAINT datev_debtor_entity_tenant_fkey FOREIGN KEY (invoicing_entity_id, tenant_id)
        REFERENCES invoicing_entity (id, tenant_id) ON DELETE CASCADE
);
CREATE INDEX datev_debtor_tenant_idx ON datev_debtor (tenant_id);
