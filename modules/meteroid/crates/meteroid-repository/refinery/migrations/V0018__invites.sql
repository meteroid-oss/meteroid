-- Modify "tenant" table
ALTER TABLE "tenant" DROP COLUMN "invite_link_id";
-- Modify "organization" table
ALTER TABLE "organization" ADD COLUMN "invite_link_hash" text NULL;
-- Create index "invite_link_tenant_id_key" to table: "invite_link"
CREATE UNIQUE INDEX "organization_invite_link_hash_idx" ON "organization" ("invite_link_hash");
-- Drop "tenant_invite" table
DROP TABLE "tenant_invite";
-- Drop "tenant_invite_link" table
DROP TABLE "tenant_invite_link";
