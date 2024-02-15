--: WebhookOutEndpoint(description?)

--! create_endpoint (description?) : WebhookOutEndpoint
insert into webhook_out_endpoint (id, tenant_id, url, description, secret, events_to_listen, enabled)
values (:id, :tenant_id, :url, :description, :secret, :events_to_listen, :enabled)
returning id, tenant_id, url, description, secret, created_at, events_to_listen, enabled;

--! list_endpoints(): WebhookOutEndpoint
select id, tenant_id, url, description, secret, created_at, events_to_listen, enabled
from webhook_out_endpoint
where tenant_id = :tenant_id;

--! get_by_id_and_tenant(): WebhookOutEndpoint
select id, tenant_id, url, description, secret, created_at, events_to_listen, enabled
from webhook_out_endpoint
where id = :id and tenant_id = :tenant_id
limit 1;
