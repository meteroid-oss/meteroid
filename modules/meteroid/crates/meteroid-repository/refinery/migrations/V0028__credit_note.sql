
CREATE TYPE "InvoiceType" as ENUM ('RECURRING', 'ONE_OFF', 'ADJUSTMENT', 'IMPORTED', 'USAGE_THRESHOLD');

-- finalized invoice should have static references
ALTER TABLE "invoice"
    ADD COLUMN "plan_version_id" uuid references plan_version on update cascade on delete restrict,
    ADD COLUMN invoice_type "InvoiceType" NOT NULL default 'RECURRING',
    ADD COLUMN "finalized_at"    TIMESTAMP(3);

ALTER TABLE "subscription"
    ADD COLUMN "activated_at"   TIMESTAMP(3);

CREATE TYPE "CreditNoteStatus" as ENUM ('DRAFT', 'FINALIZED', 'VOIDED');

CREATE TABLE "credit_note"
(
    "id"                    uuid primary key,
    "created_at"            TIMESTAMP(3)       NOT NULL,
    "updated_at"            TIMESTAMP(3)       NOT NULL,
    "refunded_amount_cents" BIGINT,
    "credited_amount_cents" BIGINT,
    "currency"              TEXT NOT NULL,
    "finalized_at"          TIMESTAMP(3)       NOT NULL,
    "plan_version_id"       uuid               NULL references plan_version on update cascade on delete set null,
    "invoice_id"            uuid               NOT NULL references invoice on update cascade on delete restrict,
    "tenant_id"             uuid               NOT NULL references tenant on update cascade on delete restrict,
    "customer_id"           uuid               NOT NULL references customer on update cascade on delete restrict,
    "status"                "CreditNoteStatus" NOT NULL
);

