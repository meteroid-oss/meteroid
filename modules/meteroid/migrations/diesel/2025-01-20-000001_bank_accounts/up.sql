 
create type "BankAccountFormat" as enum ('IBAN_BIC_SWIFT', 'ACCOUNT_ROUTING', 'SORT_CODE_ACCOUNT', 'ACCOUNT_BIC_SWIFT');
 
CREATE TABLE "bank_account"
(
  "id"              UUID                                NOT NULL PRIMARY KEY,
  "local_id"        TEXT                                NOT NULL,
  "tenant_id"       UUID                                NOT NULL REFERENCES "tenant" ON DELETE RESTRICT,
  "currency"        TEXT                                NOT NULL,
  "country"         TEXT                                NOT NULL,
  "bank_name"       TEXT                                NOT NULL,
  "format"          "BankAccountFormat"                 NOT NULL,
  "account_numbers" TEXT                                NOT NULL,
  "created_by"      UUID                                NOT NULL,
  "created_at"      TIMESTAMP default CURRENT_TIMESTAMP not null
);
ALTER TABLE "bank_account"
  ADD UNIQUE ("tenant_id", "local_id");


ALTER TABLE "invoicing_entity"
  ADD COLUMN "cc_provider_id"  UUID REFERENCES "provider_config" ON DELETE SET NULL,
  ADD COLUMN "bank_account_id" UUID REFERENCES "bank_account" ON DELETE SET NULL;
 