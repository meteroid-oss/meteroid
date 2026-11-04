DROP TABLE IF EXISTS sent_email;
DROP TABLE IF EXISTS entity_activity;
DROP TYPE IF EXISTS "ActorTypeEnum";

CREATE TABLE IF NOT EXISTS quote_activity (
    id uuid PRIMARY KEY,
    quote_id uuid NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    activity_type varchar NOT NULL,
    description text NOT NULL,
    actor_type varchar NOT NULL,
    actor_id varchar,
    actor_name varchar,
    created_at timestamptz NOT NULL DEFAULT now(),
    ip_address varchar,
    user_agent text
);

CREATE INDEX IF NOT EXISTS idx_quote_activity_quote_id ON quote_activity (quote_id, created_at DESC);
