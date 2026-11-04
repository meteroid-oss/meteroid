-- The anonymous instance report (ADR 0010): a random identity and when the last report went out.
CREATE TABLE instance_telemetry
(
    id            BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    instance_id   UUID      NOT NULL,
    installed_at  TIMESTAMP NOT NULL DEFAULT now(),
    first_version TEXT      NOT NULL,
    last_sent_at  TIMESTAMP
);
