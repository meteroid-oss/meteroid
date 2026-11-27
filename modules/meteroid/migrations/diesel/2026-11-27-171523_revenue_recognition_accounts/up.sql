-- Revenue recognition (EE module): the general ledger accounts of each invoicing entity, the account
-- each posting role goes to, and how the entity posts (ADR-0014).
CREATE TYPE "LedgerAccountRoleEnum" AS ENUM (
    'ACCOUNTS_RECEIVABLE',
    'UNBILLED_RECEIVABLE',
    'DEFERRED_REVENUE',
    'REVENUE',
    'CREDIT_LIABILITY',
    'BREAKAGE_REVENUE',
    'CONTRA_REVENUE',
    'DISPUTE_LOSS',
    'TAX_PAYABLE',
    'CASH_CLEARING',
    'BILLING_CLEARING',
    'BAD_DEBT',
    'FX_GAIN',
    'FX_LOSS',
    'ROUNDING',
    'OPENING_BALANCE'
);
CREATE TYPE "RevrecPostingScopeEnum" AS ENUM ('DOCUMENTS', 'REVENUE', 'FULL');

-- Targets of the composite foreign keys that keep each row inside its entity's tenant.
ALTER TABLE invoicing_entity ADD CONSTRAINT invoicing_entity_id_tenant_key UNIQUE (id, tenant_id);
ALTER TABLE product ADD CONSTRAINT product_id_tenant_key UNIQUE (id, tenant_id);

CREATE TABLE ledger_account (
    id                  UUID PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    invoicing_entity_id UUID        NOT NULL REFERENCES invoicing_entity (id) ON DELETE CASCADE,
    code                TEXT        NOT NULL CHECK (length(code) BETWEEN 1 AND 40 AND code = btrim(code)),
    name                TEXT        NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    archived_at         TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT ledger_account_entity_key UNIQUE (id, invoicing_entity_id),
    CONSTRAINT ledger_account_entity_tenant_fkey FOREIGN KEY (invoicing_entity_id, tenant_id)
        REFERENCES invoicing_entity (id, tenant_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX ledger_account_code_key ON ledger_account (invoicing_entity_id, code) WHERE archived_at IS NULL;
CREATE INDEX ledger_account_tenant_idx ON ledger_account (tenant_id, invoicing_entity_id);

-- The account a role posts to, and a product's own deferred revenue account. A product's own revenue
-- account is its `product_accounting.ledger_account_code`, matched to the chart by code.
CREATE TABLE ledger_account_mapping (
    id                  UUID PRIMARY KEY,
    tenant_id           UUID                    NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    invoicing_entity_id UUID                    NOT NULL REFERENCES invoicing_entity (id) ON DELETE CASCADE,
    role                "LedgerAccountRoleEnum" NOT NULL,
    product_id          UUID REFERENCES product (id) ON DELETE CASCADE,
    ledger_account_id   UUID                    NOT NULL,
    updated_at          TIMESTAMPTZ             NOT NULL DEFAULT now(),
    CONSTRAINT ledger_account_mapping_account_fkey FOREIGN KEY (ledger_account_id, invoicing_entity_id)
        REFERENCES ledger_account (id, invoicing_entity_id) ON DELETE RESTRICT,
    CONSTRAINT ledger_account_mapping_entity_tenant_fkey FOREIGN KEY (invoicing_entity_id, tenant_id)
        REFERENCES invoicing_entity (id, tenant_id) ON DELETE CASCADE,
    CONSTRAINT ledger_account_mapping_product_tenant_fkey FOREIGN KEY (product_id, tenant_id)
        REFERENCES product (id, tenant_id) ON DELETE CASCADE,
    CONSTRAINT ledger_account_mapping_product_role
        CHECK (product_id IS NULL OR role = 'DEFERRED_REVENUE')
);
CREATE UNIQUE INDEX ledger_account_mapping_key
    ON ledger_account_mapping (invoicing_entity_id, role, product_id) NULLS NOT DISTINCT;
CREATE INDEX ledger_account_mapping_account_idx ON ledger_account_mapping (ledger_account_id);

-- An unset posting scope was never chosen: nothing posts until it is. Periods are months, so the
-- books start on the first day of one.
CREATE TABLE revrec_settings (
    invoicing_entity_id   UUID PRIMARY KEY REFERENCES invoicing_entity (id) ON DELETE CASCADE,
    tenant_id             UUID                     NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    posting_scope         "RevrecPostingScopeEnum",
    accrue_unbilled_usage BOOLEAN                  NOT NULL DEFAULT TRUE,
    cutover_date          DATE CHECK (extract(day FROM cutover_date) = 1 AND cutover_date >= DATE '2000-01-01'),
    updated_at            TIMESTAMPTZ              NOT NULL DEFAULT now(),
    CONSTRAINT revrec_settings_entity_tenant_fkey FOREIGN KEY (invoicing_entity_id, tenant_id)
        REFERENCES invoicing_entity (id, tenant_id) ON DELETE CASCADE
);
CREATE INDEX revrec_settings_tenant_idx ON revrec_settings (tenant_id);
