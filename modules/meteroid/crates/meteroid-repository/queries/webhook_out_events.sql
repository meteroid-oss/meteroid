--: WebhookOutEvent(response_body?, http_status_code?, error_message?)
--: ListWebhookOutEvent(response_body?, http_status_code?, error_message?)

--! create_event(response_body?, http_status_code?, error_message?): WebhookOutEvent
insert into webhook_out_event(id, endpoint_id, event_type, request_body, response_body, http_status_code, error_message)
values (:id, :endpoint_id, :event_type, :request_body, :response_body, :http_status_code, :error_message)
returning id, endpoint_id, event_type, request_body, response_body, http_status_code, created_at, error_message;

--! list_events(): ListWebhookOutEvent
SELECT id,
       endpoint_id,
       event_type,
       request_body,
       response_body,
       http_status_code,
       created_at,
       error_message,
       COUNT(*) OVER () AS total_count
FROM webhook_out_event
WHERE endpoint_id = :endpoint_id
ORDER BY CASE
           WHEN :order_by = 'DATE_DESC' THEN created_at
           END DESC,
         CASE
           WHEN :order_by = 'DATE_ASC' THEN created_at
           END ASC,
         CASE
           WHEN :order_by = 'ID_DESC' THEN id
           END DESC,
         CASE
           WHEN :order_by = 'ID_ASC' THEN id
           END ASC
LIMIT :limit OFFSET :offset;
