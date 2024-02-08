alter table webhook_event rename to webhook_in_event;

create type "WebhookOutEventTypeEnum" as enum ('CUSTOMER_CREATED', 'SUBSCRIPTION_CREATED', 'INVOICE_CREATED', 'INVOICE_FINALIZED');

create table webhook_out_endpoint (
    id uuid not null,
    tenant_id uuid not null references tenant on delete restrict on update cascade,
    url text not null,
    description text,
    secret text not null,
    created_at timestamp(3) default CURRENT_TIMESTAMP not null,
    events_to_listen "WebhookOutEventTypeEnum"[] not null,
    enabled boolean not null,
    primary key (id)
);

create index webhook_out_endpoint_tenant_id_idx on webhook_out_endpoint(tenant_id);
