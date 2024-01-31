-- Create enum type "PlanTypeEnum"
CREATE TYPE "PlanTypeEnum" AS ENUM ('STANDARD', 'FREE', 'CUSTOM');
-- Create enum type "PlanStatusEnum"
CREATE TYPE "PlanStatusEnum" AS ENUM ('DRAFT', 'ACTIVE', 'INACTIVE', 'ARCHIVED');
-- Modify "product_family" table
ALTER TABLE "product_family" RENAME COLUMN "api_name" TO "external_id";
-- Create index "product_family_external_id_tenant_id_key" to table: "product_family"
CREATE UNIQUE INDEX "product_family_external_id_tenant_id_key" ON "product_family" ("external_id", "tenant_id");
-- Modify "plan" table
ALTER TABLE "plan" DROP COLUMN "api_name", DROP COLUMN "is_free", DROP COLUMN "trial_duration_days", ADD COLUMN "external_id" text NOT NULL, ADD COLUMN "plan_type" "PlanTypeEnum" NOT NULL, ADD COLUMN "status" "PlanStatusEnum" NOT NULL;
-- Create index "plan_tenant_id_external_id_key" to table: "plan"
CREATE UNIQUE INDEX "plan_tenant_id_external_id_key" ON "plan" ("tenant_id", "external_id");
-- Create "plan_version" table
CREATE TABLE "plan_version" ("id" uuid NOT NULL, "is_draft_version" boolean NOT NULL, "plan_id" uuid NOT NULL, "version" integer NOT NULL DEFAULT 1, "trial_duration_days" integer NULL, "trial_fallback_plan_id" uuid NULL, PRIMARY KEY ("id"), CONSTRAINT "plan_version_plan_id_fkey" FOREIGN KEY ("plan_id") REFERENCES "plan" ("id") ON UPDATE CASCADE ON DELETE RESTRICT, CONSTRAINT "plan_version_check" CHECK (((trial_duration_days IS NULL) AND (trial_fallback_plan_id IS NULL)) OR ((trial_duration_days IS NOT NULL) AND (trial_fallback_plan_id IS NOT NULL))));
-- Create index "idx_plan_version" to table: "plan_version"
CREATE INDEX "idx_plan_version" ON "plan_version" ("plan_id", "version" DESC);
-- Create "latest_plan_version" view
CREATE VIEW "latest_plan_version" ("plan_id", "name", "external_id", "description", "created_at", "created_by", "updated_at", "archived_at", "tenant_id", "product_family_id", "plan_type", "plan_version_id", "version", "trial_duration_days", "trial_fallback_plan_id") AS SELECT p.id AS plan_id,
    p.name,
    p.external_id,
    p.description,
    p.created_at,
    p.created_by,
    p.updated_at,
    p.archived_at,
    p.tenant_id,
    p.product_family_id,
    p.plan_type,
    pv.id AS plan_version_id,
    pv.version,
    pv.trial_duration_days,
    pv.trial_fallback_plan_id
   FROM (( SELECT DISTINCT ON (plan_version.plan_id) plan_version.id,
            plan_version.is_draft_version,
            plan_version.plan_id,
            plan_version.version,
            plan_version.trial_duration_days,
            plan_version.trial_fallback_plan_id
           FROM plan_version
          WHERE (plan_version.is_draft_version = false)
          ORDER BY plan_version.plan_id, plan_version.version DESC) pv
     JOIN plan p ON ((p.id = pv.plan_id)));
