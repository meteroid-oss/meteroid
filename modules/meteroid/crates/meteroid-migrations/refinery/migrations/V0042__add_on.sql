-- Create "add_on" table
CREATE TABLE "add_on"
(
  "id"              uuid  NOT NULL,
  "name"            text  NOT NULL,
  "fee"             jsonb NOT NULL,
  "tenant_id"       uuid  NOT NULL REFERENCES tenant ON UPDATE CASCADE ON DELETE RESTRICT,
  "created_at"      TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at"      TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("id")
);

create index add_on_tenant_id_idx on add_on(tenant_id);
