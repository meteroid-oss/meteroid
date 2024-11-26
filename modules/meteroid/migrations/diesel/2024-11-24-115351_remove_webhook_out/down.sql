create type "WebhookOutEventTypeEnum" as enum ('CUSTOMER_CREATED', 'SUBSCRIPTION_CREATED', 'INVOICE_CREATED', 'INVOICE_FINALIZED');

create table if not exists webhook_out_endpoint
(
  id               uuid                                   not null
    primary key,
  tenant_id        uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  url              text                                   not null,
  description      text,
  secret           text                                   not null,
  created_at       timestamp(3) default CURRENT_TIMESTAMP not null,
  events_to_listen "WebhookOutEventTypeEnum"[]            not null,
  enabled          boolean                                not null
);

create index if not exists webhook_out_endpoint_tenant_id_idx
  on webhook_out_endpoint (tenant_id);

create table if not exists webhook_out_event
(
  id               uuid                                   not null
    primary key,
  endpoint_id      uuid                                   not null
    references webhook_out_endpoint
      on update cascade on delete restrict,
  created_at       timestamp(3) default CURRENT_TIMESTAMP not null,
  event_type       "WebhookOutEventTypeEnum"              not null,
  request_body     text                                   not null,
  response_body    text,
  http_status_code smallint,
  error_message    text
);

create index if not exists webhook_out_event_endpoint_id_timestamp_idx
  on webhook_out_event (endpoint_id asc, created_at desc);
