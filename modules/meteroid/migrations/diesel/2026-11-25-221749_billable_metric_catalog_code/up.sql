-- Public API v2 (ADR-0013 §3): a metric gets a `catalog_code` of its own, unique per tenant
-- (archived metrics included) and immutable, which v2 names it by. Its `code` stays the code its
-- usage events carry: several metrics may read one event code, each aggregating it its own way.
-- No metric is changed or archived here; v1, gRPC and metering keep reading `code`.
-- A long transaction of a serving pod fails this run instead of queueing every query behind it;
-- the next start retries.
SET LOCAL lock_timeout = '5s';

-- What a metric's catalog code is made from: its event code when that is a code (as v2 writes
-- them: lowercase letters, digits, '-' and '_', starting with a letter or a digit, at most 64,
-- never reading as a metric id), else the event code's slug.
CREATE FUNCTION metric_catalog_code_base(event_code TEXT) RETURNS TEXT
    LANGUAGE sql
    IMMUTABLE AS
$$
SELECT CASE
           WHEN event_code ~ '^[a-z0-9][a-z0-9_-]{0,63}$'
               AND left(event_code, 3) <> 'bm_'
               AND event_code !~ '^[0-9a-f]{8}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{12}$'
               THEN event_code
           ELSE catalog_code_slug(event_code, 'metric')
           END
$$;

-- `unique_catalog_code` for metrics, which hold their code in `catalog_code`: `base`, or
-- `base-2`, `base-3`… the first no metric of the tenant holds and no other transaction is
-- choosing or setting, passed over rather than waited for. The lock of the code returned is held
-- until this transaction ends.
CREATE FUNCTION unique_metric_catalog_code(tenant UUID, base TEXT) RETURNS TEXT
    LANGUAGE plpgsql AS
$$
DECLARE
    candidate TEXT := base;
    n         INT  := 1;
BEGIN
    LOOP
        IF NOT EXISTS (SELECT 1 FROM billable_metric WHERE tenant_id = tenant AND catalog_code = candidate)
            AND pg_try_advisory_xact_lock(catalog_code_lock('billable_metric'::REGCLASS, tenant, candidate)) THEN
            -- Probed again: the lock's last holder may have committed the code since the probe.
            EXIT WHEN NOT EXISTS (SELECT 1
                                  FROM billable_metric
                                  WHERE tenant_id = tenant
                                    AND catalog_code = candidate);
        END IF;
        n := n + 1;
        candidate := base || '-' || n;
    END LOOP;
    RETURN candidate;
END
$$;

-- A metric inserted without a catalog code gets its event code's, unique in its tenant: every
-- insert path written before catalog codes keeps working, and a filled code never waits. One
-- inserted with its code (API v2) takes the code's lock, waiting only for a transaction choosing
-- or holding that same code, which the unique index would make it wait for anyway; so a filled
-- code never takes one an explicit insert holds. Nothing updates a catalog code: the trigger is
-- on insert only.
CREATE FUNCTION fill_metric_catalog_code() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    IF NEW.catalog_code IS NULL THEN
        NEW.catalog_code := unique_metric_catalog_code(NEW.tenant_id, metric_catalog_code_base(NEW.code));
    ELSE
        PERFORM pg_advisory_xact_lock(catalog_code_lock(TG_RELID::REGCLASS, NEW.tenant_id, NEW.catalog_code));
    END IF;
    RETURN NEW;
END
$$;

ALTER TABLE billable_metric ADD COLUMN catalog_code TEXT;

-- Before the backfill, so each probe of its leftovers is an index lookup (NULLs never collide).
CREATE UNIQUE INDEX billable_metric_tenant_catalog_code_key ON billable_metric (tenant_id, catalog_code);

-- Per tenant and base, live metrics first, then archived, oldest first: the first keeps the base
-- bare, the others are numbered `-2`, `-3`… A numbered code that is another base bare (two
-- `calls` beside a `calls-2`) is left to `unique_metric_catalog_code`, the next free number.
WITH based AS (SELECT id,
                      tenant_id,
                      metric_catalog_code_base(code)                                 AS base,
                      row_number() OVER (PARTITION BY tenant_id, metric_catalog_code_base(code)
                          ORDER BY archived_at IS NOT NULL, created_at, id)         AS n
               FROM billable_metric)
UPDATE billable_metric AS target
SET catalog_code = CASE WHEN based.n = 1 THEN based.base ELSE based.base || '-' || based.n END
FROM based
WHERE target.id = based.id
  AND (based.n = 1
    OR NOT EXISTS (SELECT 1
                   FROM based bare
                   WHERE bare.tenant_id = based.tenant_id
                     AND bare.n = 1
                     AND bare.base = based.base || '-' || based.n));

DO
$$
    DECLARE
        r RECORD;
    BEGIN
        FOR r IN SELECT id, tenant_id, code
                 FROM billable_metric
                 WHERE catalog_code IS NULL
                 ORDER BY tenant_id, archived_at IS NOT NULL, created_at, id
            LOOP
                UPDATE billable_metric
                SET catalog_code = unique_metric_catalog_code(r.tenant_id, metric_catalog_code_base(r.code))
                WHERE id = r.id;
            END LOOP;
    END
$$;

ALTER TABLE billable_metric ALTER COLUMN catalog_code SET NOT NULL;

CREATE TRIGGER trg_billable_metric_fill_catalog_code
    BEFORE INSERT ON billable_metric
    FOR EACH ROW EXECUTE FUNCTION fill_metric_catalog_code();
