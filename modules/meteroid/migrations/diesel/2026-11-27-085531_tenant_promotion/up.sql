-- Whether the invoice was finalized while its tenant was a production tenant, so a later
-- change of environment never rewrites revenue already counted. NULL: finalized before this column.
ALTER TABLE invoice ADD COLUMN production_revenue BOOLEAN;

CREATE FUNCTION invoice_set_production_revenue() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'UPDATE' AND OLD.finalized_at IS NOT NULL THEN
    NEW.production_revenue := OLD.production_revenue;
  ELSIF NEW.finalized_at IS NOT NULL THEN
    NEW.production_revenue := EXISTS (
      SELECT 1 FROM tenant t WHERE t.id = NEW.tenant_id AND t.environment = 'PRODUCTION'
    );
  ELSE
    NEW.production_revenue := NULL;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_invoice_set_production_revenue
  BEFORE INSERT OR UPDATE OF finalized_at, production_revenue ON invoice
  FOR EACH ROW EXECUTE FUNCTION invoice_set_production_revenue();

-- Invoices finalized before `invoice.production_revenue` existed count as production revenue
-- when their tenant was a production tenant then, whatever it has become since.
ALTER TABLE tenant ADD COLUMN legacy_invoices_production BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE tenant SET legacy_invoices_production = (environment = 'PRODUCTION');

-- Each process caches tenants and listens on this channel, so a change reaches every replica.
-- Activity stamps change often and are read stale anyway: they don't notify.
CREATE FUNCTION tenant_notify_changed() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('meteroid_tenant_changed', COALESCE(NEW.id, OLD.id)::text);
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_tenant_notify_changed
  AFTER UPDATE ON tenant
  FOR EACH ROW
  WHEN ((to_jsonb(OLD) - 'last_active_at' - 'updated_at') IS DISTINCT FROM (to_jsonb(NEW) - 'last_active_at' - 'updated_at'))
  EXECUTE FUNCTION tenant_notify_changed();

CREATE TRIGGER trg_tenant_notify_deleted
  AFTER DELETE ON tenant
  FOR EACH ROW EXECUTE FUNCTION tenant_notify_changed();
