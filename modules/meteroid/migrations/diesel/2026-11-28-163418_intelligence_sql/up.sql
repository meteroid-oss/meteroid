-- Intelligence > SQL: saved queries and the run log (who ran what, from the app or the API).
ALTER TYPE "StoredObjectKindEnum" ADD VALUE IF NOT EXISTS 'SQL_RESULT';

CREATE TABLE sql_saved_query
(
    id            UUID PRIMARY KEY,
    tenant_id     UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    name          TEXT      NOT NULL,
    description   TEXT,
    sql           TEXT      NOT NULL,
    -- Chart settings of the workbench, opaque to the backend.
    visualization JSONB     NOT NULL DEFAULT '{}'::jsonb,
    -- Default values of the query's `{name:Type}` parameters, as text.
    parameters    JSONB     NOT NULL DEFAULT '{}'::jsonb,
    created_by    UUID REFERENCES "user" (id) ON DELETE SET NULL,
    created_at    TIMESTAMP NOT NULL DEFAULT now(),
    updated_at    TIMESTAMP NOT NULL DEFAULT now()
);

CREATE INDEX idx_sql_saved_query_tenant ON sql_saved_query (tenant_id, updated_at DESC);

CREATE TABLE sql_query_run
(
    id             UUID PRIMARY KEY,
    tenant_id      UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    saved_query_id UUID REFERENCES sql_saved_query (id) ON DELETE SET NULL,
    user_id        UUID REFERENCES "user" (id) ON DELETE SET NULL,
    api_token_id   UUID REFERENCES api_token (id) ON DELETE SET NULL,
    sql            TEXT      NOT NULL,
    succeeded      BOOLEAN   NOT NULL,
    error          TEXT,
    row_count      INTEGER,
    elapsed_ms     BIGINT,
    rows_read      BIGINT,
    bytes_read     BIGINT,
    -- Recorded before the query runs; unset while it runs.
    finished_at    TIMESTAMP,
    -- Counted against the organization's quotas: false when the platform failed the run.
    charged        BOOLEAN   NOT NULL DEFAULT TRUE,
    -- More rows matched than the run returned.
    truncated      BOOLEAN   NOT NULL DEFAULT FALSE,
    -- The CSV of an export is kept for download (`sql_result/{run id}.csv`), for 7 days.
    result_stored  BOOLEAN   NOT NULL DEFAULT FALSE,
    created_at     TIMESTAMP NOT NULL DEFAULT now()
);

CREATE INDEX idx_sql_query_run_tenant ON sql_query_run (tenant_id, created_at DESC);
CREATE INDEX idx_sql_query_run_saved_query ON sql_query_run (saved_query_id);
CREATE INDEX idx_sql_query_run_unfinished ON sql_query_run (created_at) WHERE finished_at IS NULL;
CREATE INDEX idx_sql_query_run_result ON sql_query_run (created_at) WHERE result_stored;
CREATE INDEX idx_sql_query_run_created ON sql_query_run (created_at);

-- A saved query run on a schedule (UTC), its CSV emailed to the recipients.
CREATE TABLE sql_query_schedule
(
    id             UUID PRIMARY KEY,
    tenant_id      UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    saved_query_id UUID      NOT NULL UNIQUE REFERENCES sql_saved_query (id) ON DELETE CASCADE,
    frequency      TEXT      NOT NULL CHECK (frequency IN ('DAILY', 'WEEKLY', 'MONTHLY')),
    hour_utc       SMALLINT  NOT NULL CHECK (hour_utc BETWEEN 0 AND 23),
    -- ISO weekday (1 = Monday) when weekly, day of the month (1 to 28) when monthly.
    day            SMALLINT,
    recipients     TEXT[]    NOT NULL DEFAULT '{}',
    enabled        BOOLEAN   NOT NULL DEFAULT TRUE,
    next_run_at    TIMESTAMP NOT NULL,
    created_by     UUID REFERENCES "user" (id) ON DELETE SET NULL,
    created_at     TIMESTAMP NOT NULL DEFAULT now(),
    updated_at     TIMESTAMP NOT NULL DEFAULT now()
);

CREATE INDEX idx_sql_query_schedule_due ON sql_query_schedule (next_run_at) WHERE enabled;
CREATE INDEX idx_sql_query_schedule_tenant ON sql_query_schedule (tenant_id);

ALTER TABLE sql_query_run
    ADD COLUMN schedule_id UUID REFERENCES sql_query_schedule (id) ON DELETE SET NULL;
CREATE INDEX idx_sql_query_run_schedule ON sql_query_run (schedule_id, created_at DESC);

-- Secret keys that may run SQL over the REST API: none until granted.
CREATE TABLE sql_api_key
(
    api_token_id UUID PRIMARY KEY REFERENCES api_token (id) ON DELETE CASCADE,
    tenant_id    UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    created_at   TIMESTAMP NOT NULL DEFAULT now()
);
CREATE INDEX idx_sql_api_key_tenant ON sql_api_key (tenant_id);

-- Commit markers for the coupon, applied coupon and quote projections (SQL access), as on the
-- other projected tables: trigger-maintained, re-stamped on every write.
ALTER TABLE coupon ADD COLUMN store_committed_at timestamptz;
UPDATE coupon SET store_committed_at = COALESCE(updated_at, created_at);
ALTER TABLE coupon
    ALTER COLUMN store_committed_at SET NOT NULL,
    ALTER COLUMN store_committed_at SET DEFAULT clock_timestamp();
CREATE TRIGGER trg_coupon_store_committed_at
    BEFORE INSERT OR UPDATE ON coupon
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX idx_coupon_store_committed_at ON coupon (store_committed_at, id);

ALTER TABLE applied_coupon ADD COLUMN store_committed_at timestamptz;
UPDATE applied_coupon SET store_committed_at = COALESCE(last_applied_at, created_at);
ALTER TABLE applied_coupon
    ALTER COLUMN store_committed_at SET NOT NULL,
    ALTER COLUMN store_committed_at SET DEFAULT clock_timestamp();
CREATE TRIGGER trg_applied_coupon_store_committed_at
    BEFORE INSERT OR UPDATE ON applied_coupon
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX idx_applied_coupon_store_committed_at ON applied_coupon (store_committed_at, id);
-- The marker changes with every write: bookkeeping, like the use counts, for the subscription's
-- updated_at.
DROP TRIGGER trg_applied_coupon_touch ON applied_coupon;
CREATE TRIGGER trg_applied_coupon_touch
    AFTER INSERT OR UPDATE OR DELETE ON applied_coupon
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity('applied_count', 'applied_amount',
                                                             'last_applied_at',
                                                             'store_committed_at');

ALTER TABLE quote ADD COLUMN store_committed_at timestamptz;
UPDATE quote SET store_committed_at = COALESCE(updated_at, created_at);
ALTER TABLE quote
    ALTER COLUMN store_committed_at SET NOT NULL,
    ALTER COLUMN store_committed_at SET DEFAULT clock_timestamp();
CREATE TRIGGER trg_quote_store_committed_at
    BEFORE INSERT OR UPDATE ON quote
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX idx_quote_store_committed_at ON quote (store_committed_at, id);

-- Hard-deleted coupons and redemptions, so the projection writes a tombstone for each: the
-- keyset scan alone never sees a row again once it is gone. A tenant's deletion records none:
-- its ClickHouse rows are purged with it.
CREATE TABLE analytics_tombstone
(
    table_name TEXT        NOT NULL,
    id         UUID        NOT NULL,
    tenant_id  UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    deleted_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (table_name, id)
);
CREATE INDEX idx_analytics_tombstone_deleted ON analytics_tombstone (table_name, deleted_at, id);

CREATE FUNCTION analytics_tombstone_coupon() RETURNS trigger AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM tenant WHERE id = OLD.tenant_id) THEN
        INSERT INTO analytics_tombstone (table_name, id, tenant_id)
        VALUES ('coupon', OLD.id, OLD.tenant_id)
        ON CONFLICT (table_name, id) DO UPDATE SET deleted_at = clock_timestamp();
    END IF;
    RETURN OLD;
END
$$ LANGUAGE plpgsql;
CREATE TRIGGER trg_coupon_analytics_tombstone
    AFTER DELETE ON coupon
    FOR EACH ROW EXECUTE FUNCTION analytics_tombstone_coupon();

CREATE FUNCTION analytics_tombstone_applied_coupon() RETURNS trigger AS $$
DECLARE
    owner UUID := COALESCE(
        (SELECT tenant_id FROM subscription WHERE id = OLD.subscription_id),
        (SELECT tenant_id FROM coupon WHERE id = OLD.coupon_id));
BEGIN
    IF EXISTS (SELECT 1 FROM tenant WHERE id = owner) THEN
        INSERT INTO analytics_tombstone (table_name, id, tenant_id)
        VALUES ('applied_coupon', OLD.id, owner)
        ON CONFLICT (table_name, id) DO UPDATE SET deleted_at = clock_timestamp();
    END IF;
    RETURN OLD;
END
$$ LANGUAGE plpgsql;
CREATE TRIGGER trg_applied_coupon_analytics_tombstone
    AFTER DELETE ON applied_coupon
    FOR EACH ROW EXECUTE FUNCTION analytics_tombstone_applied_coupon();
