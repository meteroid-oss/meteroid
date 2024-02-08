--: WebhookInEvent(action?, error?)

--! get_webhook_in_event_by_id () : WebhookInEvent
SELECT id, received_at, action, key, processed, attempts, error, provider_config_id FROM webhook_in_event WHERE id = :id;

--! create_webhook_in_event () : WebhookInEvent
INSERT INTO webhook_in_event (id, received_at, key, provider_config_id)
VALUES (:id, :received_at, :key, :provider_config_id)
RETURNING id, received_at, action, key, processed, attempts, error, provider_config_id;


--! find_webhook_in_events_by_tenant_id ()
SELECT webhook_in_event.id, webhook_in_event.received_at, webhook_in_event.action, webhook_in_event.key, webhook_in_event.processed, webhook_in_event.attempts, webhook_in_event.error, provider_config.invoicing_provider
FROM webhook_in_event
JOIN provider_config ON provider_config.id = webhook_in_event.provider_config_id
WHERE provider_config.tenant_id = :tenant_id;
