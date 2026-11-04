-- Snapshot of resolved annexes (appendices) carried by an invoice, frozen at finalization.
ALTER TABLE "invoice"
  ADD COLUMN "annexes" JSONB NOT NULL DEFAULT '[]'::jsonb;

-- Reusable, tenant-scoped annex templates.
CREATE TYPE "InvoiceAnnexKindEnum" AS ENUM ('EVENT_TABLE', 'STATIC_TEXT');

CREATE TABLE "invoice_annex_template"
(
  "id"         UUID                                NOT NULL PRIMARY KEY,
  "tenant_id"  UUID                                NOT NULL REFERENCES "tenant" ON DELETE RESTRICT,
  "name"       TEXT                                NOT NULL,
  "kind"       "InvoiceAnnexKindEnum"              NOT NULL,
  "definition" JSONB                               NOT NULL,
  "created_at" TIMESTAMP default CURRENT_TIMESTAMP NOT NULL,
  "updated_at" TIMESTAMP
);

ALTER TABLE "invoice_annex_template"
  ADD UNIQUE ("tenant_id", "id");

-- Binding: which annex templates are emitted on invoices produced by a plan version.
CREATE TABLE "plan_version_annex_template"
(
  "plan_version_id"   UUID                                NOT NULL REFERENCES "plan_version" ON DELETE CASCADE,
  "annex_template_id" UUID                                NOT NULL REFERENCES "invoice_annex_template" ON DELETE CASCADE,
  "tenant_id"         UUID                                NOT NULL REFERENCES "tenant" ON DELETE RESTRICT,
  "created_at"        TIMESTAMP default CURRENT_TIMESTAMP NOT NULL,
  PRIMARY KEY ("plan_version_id", "annex_template_id")
);
