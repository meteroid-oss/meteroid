--: WebhookOutEndpoint(description?)

--! create_endpoint (description?) : WebhookOutEndpoint
insert into webhook_out_endpoint (id, tenant_id, url, description, secret, events_to_listen, enabled)
values (:id, :tenant_id, :url, :description, :secret, :events_to_listen, :enabled)
returning id, tenant_id, url, description, secret, created_at, events_to_listen, enabled;

--! list_endpoints(): WebhookOutEndpoint
select id, tenant_id, url, description, secret, created_at, events_to_listen, enabled
from webhook_out_endpoint
where tenant_id = :tenant_id;
