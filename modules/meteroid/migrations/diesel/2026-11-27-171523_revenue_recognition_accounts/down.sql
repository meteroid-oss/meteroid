DROP TABLE revrec_settings;
DROP TABLE ledger_account_mapping;
DROP TABLE ledger_account;
DROP TYPE "RevrecPostingScopeEnum";
DROP TYPE "LedgerAccountRoleEnum";
ALTER TABLE product DROP CONSTRAINT product_id_tenant_key;
ALTER TABLE invoicing_entity DROP CONSTRAINT invoicing_entity_id_tenant_key;
