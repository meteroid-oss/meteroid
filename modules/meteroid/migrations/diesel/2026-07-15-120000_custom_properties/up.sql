-- Custom (user-defined) properties for core entities, à la HubSpot/Stripe.
--
-- A tenant defines typed properties per entity type in `custom_property_definition`
-- (the schema), and each entity carries the actual values in a `custom_properties`
-- JSONB column keyed by the definition's immutable `key`
-- (e.g. {"pdp_external_status": "rejected"}). Values are validated at write time
-- against the definitions.

CREATE TYPE "CustomPropertyEntityTypeEnum" AS ENUM (
  'CUSTOMER',
  'SUBSCRIPTION',
  'INVOICE'
);

CREATE TYPE "CustomPropertyTypeEnum" AS ENUM (
  'TEXT',
  'NUMBER',
  'BOOLEAN',
  'DATE',
  'DATETIME',
  'SINGLE_SELECT',
  'MULTI_SELECT',
  'JSON',
  'URL',
  'EMAIL'
);

CREATE TABLE "custom_property_definition"
(
  "id"            UUID                           NOT NULL PRIMARY KEY,
  "tenant_id"     UUID                           NOT NULL REFERENCES "tenant" ON DELETE RESTRICT,
  "entity_type"   "CustomPropertyEntityTypeEnum" NOT NULL,
  -- immutable machine name, unique per (tenant, entity_type); the JSONB map key on entities
  "key"           TEXT                           NOT NULL,
  "name"          TEXT                           NOT NULL,
  "description"   TEXT,
  "property_type" "CustomPropertyTypeEnum"       NOT NULL,
  -- type-specific config (select options, number/text constraints, ...)
  "config"        JSONB                          NOT NULL DEFAULT '{}'::jsonb,
  "required"      BOOLEAN                        NOT NULL DEFAULT FALSE,
  "default_value" JSONB,
  "display_order" INTEGER                        NOT NULL DEFAULT 0,
  "archived_at"   TIMESTAMP,
  "created_at"    TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL,
  "updated_at"    TIMESTAMP
);

ALTER TABLE "custom_property_definition"
  ADD UNIQUE ("tenant_id", "id");

-- at most one live definition per (tenant, entity_type, key); archived ones don't collide
CREATE UNIQUE INDEX "custom_property_definition_tenant_entity_key_idx"
  ON "custom_property_definition" ("tenant_id", "entity_type", "key")
  WHERE "archived_at" IS NULL;

CREATE INDEX "custom_property_definition_tenant_entity_idx"
  ON "custom_property_definition" ("tenant_id", "entity_type");

-- Per-entity value bags. Keyed by definition `key`; empty object when unset.
ALTER TABLE "customer"     ADD COLUMN "custom_properties" JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE "subscription" ADD COLUMN "custom_properties" JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE "invoice"      ADD COLUMN "custom_properties" JSONB NOT NULL DEFAULT '{}'::jsonb;

-- GIN indexes so analytics/workflows can filter entities by property values.
CREATE INDEX "customer_custom_properties_idx"     ON "customer"     USING GIN ("custom_properties");
CREATE INDEX "subscription_custom_properties_idx" ON "subscription" USING GIN ("custom_properties");
CREATE INDEX "invoice_custom_properties_idx"      ON "invoice"      USING GIN ("custom_properties");
