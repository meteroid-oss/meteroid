DROP INDEX idx_customer_connection_connector;
DROP INDEX idx_payment_transaction_in_flight;
DROP INDEX connector_tenant_alias_live_uidx;
DELETE FROM connector WHERE status = 'DRAFT';
-- Deleted connectors are still referenced by payment history; keep them, out of the alias namespace.
UPDATE connector SET alias = alias || '-deleted-' || id WHERE status = 'DELETED';
ALTER TABLE connector ADD CONSTRAINT connector_tenant_id_alias_key UNIQUE (tenant_id, alias);

ALTER TABLE connector
    DROP COLUMN credentials_error_at,
    DROP COLUMN credentials_error,
    DROP COLUMN webhook_route,
    DROP COLUMN account,
    DROP COLUMN updated_at,
    DROP COLUMN status;

DROP TYPE "ConnectorStatusEnum";
DROP TYPE "ConnectorWebhookRouteEnum";
