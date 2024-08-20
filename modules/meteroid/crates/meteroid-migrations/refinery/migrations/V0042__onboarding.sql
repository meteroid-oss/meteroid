drop table invoicing_config;
CREATE TABLE invoicing_entity
(
    id                      UUID PRIMARY KEY,
    local_id                text        NOT NULL,
    is_default              BOOLEAN     NOT NULL,
    legal_name              text        NOT NULL,
    invoice_number_pattern  text        NOT NULL,
    next_invoice_number     bigint      NOT NULL,
    next_credit_note_number bigint      NOT NULL,
    grace_period_hours      integer     NOT NULL,
    net_terms               integer     NOT NULL,
    invoice_footer_info     TEXT,
    invoice_footer_legal    TEXT,
    logo_attachment_id      text,
    brand_color             text,
    address_line1           text,
    address_line2           text,
    zip_code                VARCHAR(50),
    state                   text,
    city                    text,
    tax_id                  text,
    country                 text        NOT NULL,
    currency                VARCHAR(50) NOT NULL,
    tenant_id               UUID        NOT NULL,
    CONSTRAINT "invoicing_entity_tenant_id_fkey" FOREIGN KEY ("tenant_id") REFERENCES "tenant" ("id") ON UPDATE CASCADE ON DELETE CASCADE,
    CONSTRAINT "invoicing_entity_is_default_tenant_id_key" UNIQUE ("is_default", "tenant_id"),
    CONSTRAINT "invoicing_entity_local_id_tenant_id_key" UNIQUE ("local_id", "tenant_id")
);


ALTER TABLE "user"
    ADD COLUMN onboarded  BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN first_name text    NOT NULL,
    ADD COLUMN last_name  text,
    ADD COLUMN department text;

ALTER TABLE organization
    RENAME COLUMN name TO default_trade_name;
ALTER TABLE organization
    ADD COLUMN default_country text NOT NULL;

ALTER TABLE customer
    ADD COLUMN invoicing_entity_id UUID NOT NULL references invoicing_entity (id) ON DELETE RESTRICT;
