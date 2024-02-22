
CREATE TYPE "InvoiceType" as ENUM ('RECURRING', 'ONE_OFF', 'ADJUSTMENT', 'IMPORTED', 'USAGE_THRESHOLD');

-- finalized invoice should have static references, even if the subscription gets updated, as we don't version the subscription.
ALTER TABLE "invoice"
--     ADD COLUMN "plan_id"         uuid   NULL references plan on update cascade on delete restrict,
    ADD COLUMN "plan_version_id" uuid NULL references plan_version on update cascade on delete restrict,
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
    "finalized_at"          TIMESTAMP(3)       NOT NULL,
    "invoice_id"            uuid               NOT NULL references invoice on update cascade on delete restrict,
    "tenant_id"             uuid               NOT NULL references tenant on update cascade on delete restrict,
    "status"                "CreditNoteStatus" NOT NULL
);


