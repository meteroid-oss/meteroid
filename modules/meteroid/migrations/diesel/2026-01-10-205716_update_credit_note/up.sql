-- Drop the old credit_note table (it's empty and outdated)
DROP TABLE IF EXISTS credit_note CASCADE;

-- Recreate credit_note table with modern structure similar to invoice
CREATE TABLE credit_note (
    id                    uuid                    NOT NULL PRIMARY KEY,
    credit_note_number    text                    NOT NULL,
    status                "CreditNoteStatus"      NOT NULL,
    created_at            timestamptz             NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at            timestamptz,
    finalized_at          timestamptz,
    voided_at             timestamptz,

    -- Tenant and relationships
    tenant_id             uuid                    NOT NULL REFERENCES tenant ON UPDATE CASCADE ON DELETE RESTRICT,
    customer_id           uuid                    NOT NULL REFERENCES customer ON UPDATE CASCADE ON DELETE RESTRICT,
    invoice_id            uuid                    NOT NULL REFERENCES invoice ON UPDATE CASCADE ON DELETE RESTRICT,
    plan_version_id       uuid                    REFERENCES plan_version ON UPDATE CASCADE ON DELETE SET NULL,
    subscription_id       uuid                    REFERENCES subscription ON UPDATE CASCADE ON DELETE SET NULL,

    -- Amounts (in minor currency units, e.g., cents)
    currency              text                    NOT NULL,
    subtotal              bigint                  NOT NULL,
    tax_amount            bigint                  NOT NULL,
    total                 bigint                  NOT NULL,

    -- Breakdown of credit note application
    refunded_amount_cents bigint                  DEFAULT 0 NOT NULL,
    credited_amount_cents bigint                  DEFAULT 0 NOT NULL,

    -- Line items showing what's being credited
    line_items            jsonb                   NOT NULL DEFAULT '[]'::jsonb,

    -- Tax breakdown
    tax_breakdown         jsonb                   NOT NULL DEFAULT '[]'::jsonb,

    -- Reason and memo
    reason                text,
    memo                  text,

    -- Snapshot data (like invoices)
    customer_details      jsonb                   NOT NULL,
    seller_details        jsonb                   NOT NULL,

    -- Documents
    pdf_document_id       uuid,

    -- Connection metadata for external integrations
    conn_meta             jsonb,

    -- Invoicing entity
    invoicing_entity_id   uuid                    NOT NULL REFERENCES invoicing_entity ON UPDATE CASCADE ON DELETE RESTRICT
);

-- Indexes for efficient querying
CREATE INDEX idx_credit_note_tenant_id ON credit_note(tenant_id);
CREATE INDEX idx_credit_note_customer_id ON credit_note(customer_id);
CREATE INDEX idx_credit_note_invoice_id ON credit_note(invoice_id);
CREATE INDEX idx_credit_note_created_at ON credit_note(created_at DESC);

-- Unique constraint on credit note number per tenant (excluding drafts)
CREATE UNIQUE INDEX credit_note_number_key
    ON credit_note(credit_note_number, tenant_id)
    WHERE (status != 'DRAFT'::"CreditNoteStatus");

-- Update bi_mrr_movement_log to restore the foreign key
-- (it was dropped when we dropped the credit_note table)
ALTER TABLE bi_mrr_movement_log
    DROP CONSTRAINT IF EXISTS bi_mrr_movement_log_credit_note_id_fkey;

ALTER TABLE bi_mrr_movement_log
    ADD CONSTRAINT bi_mrr_movement_log_credit_note_id_fkey
    FOREIGN KEY (credit_note_id)
    REFERENCES credit_note(id)
    ON UPDATE CASCADE ON DELETE RESTRICT;
