CREATE TABLE IF NOT EXISTS "customer_balance_tx"
(
  "id"                    uuid primary key,
  "created_at"            TIMESTAMP(3)             NOT NULL default CURRENT_TIMESTAMP,
  "amount_cents"          INTEGER                  NOT NULL,
  "balance_cents_after"   INTEGER                  NOT NULL,
  "note"                  text,
  "invoice_id"            uuid                               references invoice on update cascade on delete restrict,
  "tenant_id"             uuid                     NOT NULL  references tenant on update cascade on delete restrict,
  "customer_id"           uuid                     NOT NULL  references customer on update cascade on delete restrict,
  "created_by"            uuid                               references "user" on update cascade on delete restrict
);

CREATE TABLE IF NOT EXISTS "customer_balance_pending_tx"
(
  "id"                    uuid primary key,
  "created_at"            TIMESTAMP(3)             NOT NULL default CURRENT_TIMESTAMP,
  "updated_at"            TIMESTAMP(3)             NOT NULL default CURRENT_TIMESTAMP,
  "amount_cents"          INTEGER                  NOT NULL,
  "note"                  text,
  "invoice_id"            uuid                               references invoice on update cascade on delete restrict,
  "tenant_id"             uuid                     NOT NULL  references tenant on update cascade on delete restrict,
  "customer_id"           uuid                     NOT NULL  references customer on update cascade on delete restrict,
  "tx_id"                 uuid                               references customer_balance_tx on update cascade on delete restrict,
  "created_by"            uuid                     NOT NULL  references "user" on update cascade on delete restrict
);

alter table customer
  add constraint customer_balance_non_negative check (customer.balance_value_cents >= 0);

alter table invoice
  add column "applied_credits" bigint not null default 0;
