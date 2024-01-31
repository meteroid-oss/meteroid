--: WebhookEvent(action?, error?)

--! get_webhook_event_by_id () : WebhookEvent
SELECT id, received_at, action, key, processed, attempts, error, provider_config_id FROM webhook_event WHERE id = :id;

--! create_webhook_event () : WebhookEvent
INSERT INTO webhook_event (id, received_at, key, provider_config_id)
VALUES (:id, :received_at, :key, :provider_config_id)
RETURNING id, received_at, action, key, processed, attempts, error, provider_config_id;


--! find_webhook_events_by_tenant_id ()
SELECT webhook_event.id, webhook_event.received_at, webhook_event.action, webhook_event.key, webhook_event.processed, webhook_event.attempts, webhook_event.error, provider_config.invoicing_provider
FROM webhook_event
JOIN provider_config ON provider_config.id = webhook_event.provider_config_id
WHERE provider_config.tenant_id = :tenant_id;
