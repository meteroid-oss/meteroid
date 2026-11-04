-- Connector lifecycle: drafts (multi-step setup), disabling, soft deletion.
CREATE TYPE "ConnectorStatusEnum" AS ENUM ('DRAFT', 'ACTIVE', 'DISABLED', 'DELETED');
-- Where the provider sends webhooks; NULL for connectors that receive none.
CREATE TYPE "ConnectorWebhookRouteEnum" AS ENUM ('CONNECTOR_ID', 'TENANT_ALIAS');

ALTER TABLE connector
    ADD COLUMN status             "ConnectorStatusEnum" NOT NULL DEFAULT 'ACTIVE',
    ADD COLUMN updated_at         TIMESTAMP NOT NULL DEFAULT now(),
    -- What we last learned from the provider account (id, mode, enabled payment methods).
    ADD COLUMN account            JSONB,
    -- CONNECTOR_ID: /webhooks/v1/payments/{id}. TENANT_ALIAS: /webhooks/v1/{tenant}/{alias}.
    ADD COLUMN webhook_route      "ConnectorWebhookRouteEnum",
    -- Set when the provider refused the stored credentials (revoked, rolled, expired); cleared by
    -- the next successful check. Independent of the status.
    ADD COLUMN credentials_error    TEXT,
    ADD COLUMN credentials_error_at TIMESTAMP;

-- Existing payment connectors keep the URL their provider was set up with. Stancer sends none.
UPDATE connector SET webhook_route = 'TENANT_ALIAS'
WHERE connector_type = 'PAYMENT_PROVIDER' AND provider <> 'STANCER';

-- A deleted connector frees its alias.
ALTER TABLE connector DROP CONSTRAINT connector_tenant_id_alias_key;
CREATE UNIQUE INDEX connector_tenant_alias_live_uidx
    ON connector (tenant_id, alias) WHERE status <> 'DELETED';

-- Deleting a connector counts its in-flight payments; those rows are few, so the index stays small.
CREATE INDEX idx_payment_transaction_in_flight
    ON payment_transaction (tenant_id) WHERE status IN ('PENDING', 'READY');
-- Connector usage starts from the connector's customer connections.
CREATE INDEX idx_customer_connection_connector ON customer_connection (connector_id);
