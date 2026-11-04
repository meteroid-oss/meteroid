-- Saved list "views" for the core entity list pages (customers/subscriptions/invoices), à la
-- Linear/Attio. A view is a named snapshot of a list's state — advanced filters, quick filters,
-- sort and visible columns — stored as an opaque, versioned `config` JSONB owned by the web
-- client.
--
-- Visibility: a view is private to its creator unless `shared`, in which case every member of
-- the tenant sees it (and may edit it; only the creator may toggle sharing). A private view whose
-- creator is deleted becomes unreachable (`created_by` NULL, not shared) and is inert.

CREATE TABLE "entity_list_view"
(
  "id"          UUID                           NOT NULL PRIMARY KEY,
  "tenant_id"   UUID                           NOT NULL REFERENCES "tenant" ON DELETE CASCADE,
  "entity_type" "CustomPropertyEntityTypeEnum" NOT NULL,
  "name"        TEXT                           NOT NULL,
  -- Opaque { v, filters, quick, sort, columns }, interpreted by the web client only.
  "config"      JSONB                          NOT NULL DEFAULT '{}'::jsonb,
  "shared"      BOOLEAN                        NOT NULL DEFAULT FALSE,
  "created_by"  UUID                           REFERENCES "user" ON DELETE SET NULL,
  "created_at"  TIMESTAMP                      NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at"  TIMESTAMP,
  CONSTRAINT "entity_list_view_name_chk" CHECK (length(btrim("name")) BETWEEN 1 AND 100),
  CONSTRAINT "entity_list_view_config_chk" CHECK (jsonb_typeof("config") = 'object')
);

CREATE INDEX "entity_list_view_tenant_entity_idx"
  ON "entity_list_view" ("tenant_id", "entity_type");

-- Each user's default ("open on load") view per list. Per user rather than per view, so a shared
-- view can be one member's default without being everyone's. Removed with the view or the user, and
-- for everyone but the creator when the view is made private.
CREATE TABLE "entity_list_view_default"
(
  "tenant_id"   UUID                           NOT NULL REFERENCES "tenant" ON DELETE CASCADE,
  "user_id"     UUID                           NOT NULL REFERENCES "user" ON DELETE CASCADE,
  "entity_type" "CustomPropertyEntityTypeEnum" NOT NULL,
  "view_id"     UUID                           NOT NULL REFERENCES "entity_list_view" ON DELETE CASCADE,
  PRIMARY KEY ("tenant_id", "user_id", "entity_type")
);

CREATE INDEX "entity_list_view_default_view_idx" ON "entity_list_view_default" ("view_id");
