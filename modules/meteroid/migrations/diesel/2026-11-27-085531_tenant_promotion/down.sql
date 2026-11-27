DROP TRIGGER trg_tenant_notify_deleted ON tenant;
DROP TRIGGER trg_tenant_notify_changed ON tenant;
DROP FUNCTION tenant_notify_changed();
ALTER TABLE tenant DROP COLUMN legacy_invoices_production;
DROP TRIGGER trg_invoice_set_production_revenue ON invoice;
DROP FUNCTION invoice_set_production_revenue();
ALTER TABLE invoice DROP COLUMN production_revenue;
