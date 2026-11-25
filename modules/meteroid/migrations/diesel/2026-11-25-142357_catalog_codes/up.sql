-- Public API v2 (ADR-0013): plans, products, add-ons and product families get a `code`, unique per
-- tenant and immutable, that every catalog path takes beside the id; a subscription gets an
-- optional `external_id`, unique per tenant when set.
-- A long transaction of a serving pod fails this run instead of queueing every query behind it;
-- the next start retries.
SET LOCAL lock_timeout = '5s';

-- ── codes ──

-- The slug of a name: lowercase ASCII letters and digits, any run of anything else one '-', at
-- most 56 characters (room for a suffix), `fallback` when nothing is left. Never starts with an
-- id prefix: those end with '_'.
CREATE FUNCTION catalog_code_slug(name TEXT, fallback TEXT) RETURNS TEXT
    LANGUAGE sql
    IMMUTABLE AS
$$
SELECT COALESCE(
               NULLIF(
                       trim(BOTH '-' FROM
                            left(trim(BOTH '-' FROM regexp_replace(lower(name), '[^a-z0-9]+', '-', 'g')), 56)),
                       ''),
               fallback)
$$;

-- The advisory lock a code of `tbl` in `tenant` is chosen or set under, held until the transaction
-- ends. One per code: rows of unrelated names never wait on each other.
CREATE FUNCTION catalog_code_lock(tbl REGCLASS, tenant UUID, code TEXT) RETURNS BIGINT
    LANGUAGE sql
    IMMUTABLE AS
$$
SELECT hashtextextended(tbl::OID::TEXT || ':' || tenant::TEXT || ':' || code, 0)
$$;

-- `base`, or `base-2`, `base-3`… the first no row of `tbl` of the tenant holds and no other
-- transaction is choosing or setting: a code whose lock another transaction holds is passed over,
-- never waited for. The lock of the code returned is held until this transaction ends.
CREATE FUNCTION unique_catalog_code(tbl REGCLASS, tenant UUID, base TEXT) RETURNS TEXT
    LANGUAGE plpgsql AS
$$
DECLARE
    probe     TEXT := format('SELECT EXISTS (SELECT 1 FROM %s WHERE tenant_id = $1 AND code = $2)', tbl);
    candidate TEXT := base;
    n         INT  := 1;
    taken     BOOLEAN;
BEGIN
    LOOP
        EXECUTE probe INTO taken USING tenant, candidate;
        IF NOT taken AND pg_try_advisory_xact_lock(catalog_code_lock(tbl, tenant, candidate)) THEN
            -- Probed again: the lock's last holder may have committed the code since the probe.
            EXECUTE probe INTO taken USING tenant, candidate;
            EXIT WHEN NOT taken;
        END IF;
        n := n + 1;
        candidate := base || '-' || n;
    END LOOP;
    RETURN candidate;
END
$$;

-- A row inserted without a code gets its name's, unique in its tenant: every insert path written
-- before codes keeps working. A code set explicitly (API v2 inserts the row, then sets its code in
-- the same transaction) takes the code's lock, waiting only for a transaction choosing or holding
-- that same code, which its unique index would make it wait for anyway; a code filled here never
-- waits. So two inserts of a tenant wait on each other only for one code, a filled code never takes
-- one an explicit write is setting (no unique violation for the insert that did not name it), and
-- no lock is taken in an order two transactions could invert. The argument is the fallback of a
-- name without a letter or a digit.
CREATE FUNCTION fill_catalog_code() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    IF TG_OP = 'INSERT' AND NEW.code IS NULL THEN
        NEW.code := unique_catalog_code(TG_RELID::REGCLASS, NEW.tenant_id,
                                        catalog_code_slug(NEW.name, TG_ARGV[0]));
    ELSIF NEW.code IS NOT NULL AND (TG_OP = 'INSERT' OR NEW.code IS DISTINCT FROM OLD.code) THEN
        PERFORM pg_advisory_xact_lock(catalog_code_lock(TG_RELID::REGCLASS, NEW.tenant_id, NEW.code));
    END IF;
    RETURN NEW;
END
$$;

ALTER TABLE plan ADD COLUMN code TEXT;
ALTER TABLE product ADD COLUMN code TEXT;
ALTER TABLE add_on ADD COLUMN code TEXT;
ALTER TABLE product_family ADD COLUMN code TEXT;

-- Before the backfill, so each probe of its leftovers is an index lookup (NULLs never collide).
CREATE UNIQUE INDEX plan_tenant_code_key ON plan (tenant_id, code);
CREATE UNIQUE INDEX product_tenant_code_key ON product (tenant_id, code);
CREATE UNIQUE INDEX add_on_tenant_code_key ON add_on (tenant_id, code);
CREATE UNIQUE INDEX product_family_tenant_code_key ON product_family (tenant_id, code);

-- One statement per table: the oldest row of a slug in its tenant keeps it bare, the others are
-- numbered `-2`, `-3`… in creation order. Products are created under their charge's name ("Seats",
-- "Platform fee"), so a tenant can hold thousands of one name: probing each row's code in turn
-- would cost the square of that count. A numbered code that is another name's bare slug (a `Pro 2`
-- beside two `Pro`) is left to `unique_catalog_code`, the next free number: a few rows, each
-- probing past its slug's numbered ones.
DO
$$
    DECLARE
        t RECORD;
        r RECORD;
    BEGIN
        FOR t IN SELECT *
                 FROM (VALUES ('plan', 'plan'),
                              ('product', 'product'),
                              ('add_on', 'add-on'),
                              ('product_family', 'family')) AS v (tbl, fallback)
            LOOP
                EXECUTE format($sql$
                    WITH slugged AS (SELECT id, tenant_id, created_at, catalog_code_slug(name, %L) AS base
                                     FROM %I),
                         numbered AS (SELECT id, tenant_id, base,
                                             row_number() OVER (PARTITION BY tenant_id, base
                                                 ORDER BY created_at, id) AS n
                                      FROM slugged)
                    UPDATE %I AS target
                    SET code = CASE WHEN numbered.n = 1 THEN numbered.base
                                    ELSE numbered.base || '-' || numbered.n END
                    FROM numbered
                    WHERE target.id = numbered.id
                      AND (numbered.n = 1
                        OR NOT EXISTS (SELECT 1
                                       FROM numbered bare
                                       WHERE bare.tenant_id = numbered.tenant_id
                                         AND bare.n = 1
                                         AND bare.base = numbered.base || '-' || numbered.n))
                    $sql$, t.fallback, t.tbl, t.tbl);
                FOR r IN EXECUTE format('SELECT id, tenant_id, name FROM %I WHERE code IS NULL
                                         ORDER BY tenant_id, created_at, id', t.tbl)
                    LOOP
                        EXECUTE format('UPDATE %I SET code = unique_catalog_code(%L::REGCLASS, $1,
                                            catalog_code_slug($2, %L)) WHERE id = $3',
                                       t.tbl, t.tbl, t.fallback)
                            USING r.tenant_id, r.name, r.id;
                    END LOOP;
            END LOOP;
    END
$$;

ALTER TABLE plan ALTER COLUMN code SET NOT NULL;
ALTER TABLE product ALTER COLUMN code SET NOT NULL;
ALTER TABLE add_on ALTER COLUMN code SET NOT NULL;
ALTER TABLE product_family ALTER COLUMN code SET NOT NULL;

CREATE TRIGGER trg_plan_fill_code
    BEFORE INSERT OR UPDATE OF code ON plan
    FOR EACH ROW EXECUTE FUNCTION fill_catalog_code('plan');
CREATE TRIGGER trg_product_fill_code
    BEFORE INSERT OR UPDATE OF code ON product
    FOR EACH ROW EXECUTE FUNCTION fill_catalog_code('product');
CREATE TRIGGER trg_add_on_fill_code
    BEFORE INSERT OR UPDATE OF code ON add_on
    FOR EACH ROW EXECUTE FUNCTION fill_catalog_code('add-on');
CREATE TRIGGER trg_product_family_fill_code
    BEFORE INSERT OR UPDATE OF code ON product_family
    FOR EACH ROW EXECUTE FUNCTION fill_catalog_code('family');

-- ── subscription external id ──

ALTER TABLE subscription ADD COLUMN external_id TEXT;
-- Built in the transaction, not CONCURRENTLY (this migration runs inside the release's transaction,
-- `READ_BY_THE_CONVERSION`): every external id is NULL yet, so the build scans the table (about
-- 20,000 rows in the largest installation) and indexes nothing, well under a second.
CREATE UNIQUE INDEX subscription_tenant_external_id_key
    ON subscription (tenant_id, external_id)
    WHERE external_id IS NOT NULL;

-- ── idempotent responses ──

-- A v2 write keyed by its Idempotency-Key keeps its first response, replayed to a retry. NULL
-- while the first is running, and on v1 keys (they derive what they made from the key instead).
ALTER TABLE api_idempotency_key
    ADD COLUMN response_status INT,
    ADD COLUMN response_body   BYTEA;
