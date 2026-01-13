-- Drop the modern credit_note table
DROP TABLE IF EXISTS credit_note CASCADE;

-- Restore the old simple credit_note table structure
CREATE TABLE credit_note (
    id                    uuid               NOT NULL PRIMARY KEY,
    created_at            timestamp(3)       NOT NULL,
    updated_at            timestamp(3)       NOT NULL,
    refunded_amount_cents bigint,
    credited_amount_cents bigint,
    currency              text               NOT NULL,
    finalized_at          timestamp(3)       NOT NULL,
    plan_version_id       uuid               REFERENCES plan_version ON UPDATE CASCADE ON DELETE SET NULL,
    invoice_id            uuid               NOT NULL REFERENCES invoice ON UPDATE CASCADE ON DELETE RESTRICT,
    tenant_id             uuid               NOT NULL REFERENCES tenant ON UPDATE CASCADE ON DELETE RESTRICT,
    customer_id           uuid               NOT NULL REFERENCES customer ON UPDATE CASCADE ON DELETE RESTRICT,
    status                "CreditNoteStatus" NOT NULL
);

-- Restore bi_mrr_movement_log foreign key
ALTER TABLE bi_mrr_movement_log
    DROP CONSTRAINT IF EXISTS bi_mrr_movement_log_credit_note_id_fkey;

ALTER TABLE bi_mrr_movement_log
    ADD CONSTRAINT bi_mrr_movement_log_credit_note_id_fkey
    FOREIGN KEY (credit_note_id)
    REFERENCES credit_note(id)
    ON UPDATE CASCADE ON DELETE RESTRICT;
