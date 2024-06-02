create table if not exists webhook_out_event (
    id uuid not null primary key,
    endpoint_id uuid not null references webhook_out_endpoint on delete restrict on update cascade,
    created_at timestamp(3) default CURRENT_TIMESTAMP not null,
    event_type "WebhookOutEventTypeEnum" not null,
    request_body text not null,
    response_body text,
    http_status_code smallint,
    error_message text
);

create index if not exists webhook_out_event_endpoint_id_timestamp_idx on webhook_out_event(endpoint_id, created_at desc);
