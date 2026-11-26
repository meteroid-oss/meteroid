-- An event's payload rendered at an API version that has payloads of its own (ADR-0013 §15),
-- once per version its endpoints hold, when the event is fanned out: what every delivery and
-- retry at that version sends. `webhook_out_message.payload` stays v1's, which a delivery
-- without a version sends.
SET LOCAL lock_timeout = '5s';

CREATE TABLE webhook_out_message_version (
    message_id UUID NOT NULL REFERENCES webhook_out_message (id) ON DELETE CASCADE,
    api_version TEXT NOT NULL,
    payload JSONB NOT NULL,
    PRIMARY KEY (message_id, api_version)
);

-- The version a delivery's payload was rendered at: NULL for v1's payload, else a row of
-- `webhook_out_message_version`. Set when the delivery is created, never changed: a retry or a
-- resend sends the same bytes, whatever the endpoint holds since.
ALTER TABLE webhook_out_delivery ADD COLUMN api_version TEXT;

ALTER TABLE webhook_out_delivery
    ADD CONSTRAINT webhook_out_delivery_message_version_fkey
    FOREIGN KEY (message_id, api_version)
    REFERENCES webhook_out_message_version (message_id, api_version) ON DELETE CASCADE;
