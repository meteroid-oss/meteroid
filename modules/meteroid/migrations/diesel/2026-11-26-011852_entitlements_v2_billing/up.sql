-- Entitlements v2, phase 4 (ADR 0011 §3.5, §3.6): subscriptions, quotes and checkouts for one
-- resource, plans sold for a type, metrics attributed to one, and where a fee row a late change
-- wrote bills its metric from. Leftover usage filters become resources; a subscription's claim on
-- usage is read from its start and ending facts. A lock not granted soon fails the migration, to
-- be rerun, rather than deadlock.
SET LOCAL lock_timeout = '10s';
LOCK TABLE subscription IN SHARE ROW EXCLUSIVE MODE;

-- The columns may exist already: the release billing from facts adds them bare before writing the
-- changes left pending, which reads them (meteroid-app `migrations`).
ALTER TABLE subscription
    ADD COLUMN IF NOT EXISTS resource_id UUID,
    ADD CONSTRAINT subscription_resource_fkey FOREIGN KEY (customer_id, resource_id)
        REFERENCES resource (customer_id, id) ON DELETE RESTRICT;
CREATE INDEX subscription_resource_idx ON subscription (resource_id) WHERE resource_id IS NOT NULL;

ALTER TABLE quote
    ADD COLUMN IF NOT EXISTS resource_id UUID,
    ADD CONSTRAINT quote_resource_fkey FOREIGN KEY (customer_id, resource_id)
        REFERENCES resource (customer_id, id) ON DELETE RESTRICT;
CREATE INDEX quote_resource_idx ON quote (resource_id) WHERE resource_id IS NOT NULL;

ALTER TABLE checkout_session
    ADD COLUMN IF NOT EXISTS resource_id UUID,
    ADD CONSTRAINT checkout_session_resource_fkey FOREIGN KEY (customer_id, resource_id)
        REFERENCES resource (customer_id, id) ON DELETE RESTRICT;

ALTER TABLE plan
    ADD COLUMN IF NOT EXISTS sold_for_resource_type_id UUID,
    ADD CONSTRAINT plan_sold_for_resource_type_id_fkey FOREIGN KEY (sold_for_resource_type_id)
        REFERENCES resource_type (id);

-- NULL property: the first of the type's keys present on the event, canonical first.
ALTER TABLE billable_metric
    ADD COLUMN IF NOT EXISTS attributed_to_type_id UUID,
    ADD COLUMN IF NOT EXISTS attribution_property  TEXT,
    ADD CONSTRAINT billable_metric_attributed_to_type_id_fkey FOREIGN KEY (attributed_to_type_id)
        REFERENCES resource_type (id),
    ADD CONSTRAINT billable_metric_attribution_check
        CHECK (attribution_property IS NULL OR attributed_to_type_id IS NOT NULL);

-- Leftover usage filters become resources: one type per property, keyed by the property as
-- events carry it, one resource per value, and the metrics their subscriptions price attributed
-- to the type. Its code is a slug of the property (paths and attributes are spelled with it).
CREATE TEMP TABLE usage_filter_type ON COMMIT DROP AS
WITH slug AS (SELECT p.tenant_id,
                     p.property,
                     k.resource_type_id,
                     left(CASE
                              WHEN p.s = '' THEN 'resource'
                              WHEN p.s ~ '^[a-z]' THEN p.s
                              ELSE 'r_' || p.s END, 60) AS base
              FROM (SELECT DISTINCT tenant_id,
                                    usage_filter_property AS property,
                                    btrim(regexp_replace(lower(usage_filter_property), '[^a-z0-9_]+', '_', 'g'),
                                          '_')        AS s
                    FROM subscription
                    WHERE usage_filter_property IS NOT NULL) p
                       LEFT JOIN resource_type_key k ON k.tenant_id = p.tenant_id AND k.key = p.property)
SELECT tenant_id,
       property,
       COALESCE(resource_type_id, gen_random_uuid()) AS resource_type_id,
       resource_type_id IS NULL                      AS created,
       CASE
           WHEN row_number() OVER w = 1 THEN base
           ELSE base || '_' || row_number() OVER w END AS code
FROM slug
WINDOW w AS (PARTITION BY tenant_id, base ORDER BY property = base DESC, property);

INSERT INTO resource_type (id, tenant_id, code, name, plural_name)
SELECT resource_type_id,
       tenant_id,
       code,
       COALESCE(NULLIF(btrim(left(property, 128)), ''), code),
       COALESCE(NULLIF(btrim(left(property, 128)), ''), code)
FROM usage_filter_type
WHERE created
ON CONFLICT (tenant_id, code) DO NOTHING;

-- A type whose code was taken was not created: its filters stop the migration below.
INSERT INTO resource_type_key (tenant_id, resource_type_id, key, position)
SELECT u.tenant_id, u.resource_type_id, u.property, 0
FROM usage_filter_type u
         JOIN resource_type rt ON rt.id = u.resource_type_id
WHERE u.created
ON CONFLICT DO NOTHING;

CREATE TEMP TABLE usage_filter_conversion ON COMMIT DROP AS
SELECT s.id AS subscription_id, s.tenant_id, s.customer_id, k.resource_type_id, s.usage_filter_value AS key_value
FROM subscription s
         JOIN resource_type_key k ON k.tenant_id = s.tenant_id AND k.key = s.usage_filter_property
WHERE s.usage_filter_value IS NOT NULL;

INSERT INTO resource (id, tenant_id, customer_id, resource_type_id, key_value)
SELECT gen_random_uuid(), tenant_id, customer_id, resource_type_id, key_value
FROM (SELECT DISTINCT tenant_id, customer_id, resource_type_id, key_value FROM usage_filter_conversion) c
ON CONFLICT (tenant_id, customer_id, resource_type_id, key_value) DO NOTHING;

-- The same subscriptions under another name, not a change an API reader is told of: as a
-- backfill, it keeps their `updated_at` and activity.
ALTER TABLE subscription DISABLE TRIGGER trg_subscription_touch;
UPDATE subscription s
SET resource_id = r.id
FROM usage_filter_conversion c
         JOIN resource r ON r.tenant_id = c.tenant_id AND r.customer_id = c.customer_id
    AND r.resource_type_id = c.resource_type_id AND r.key_value = c.key_value
WHERE s.id = c.subscription_id;
ALTER TABLE subscription ENABLE TRIGGER trg_subscription_touch;

-- The metrics converted subscriptions price, by a component or an add-on, with their filter's type.
-- A row's structure is its charge's (its plan component's, its add-on's), else its stored fee's,
-- as billing reads it; its plan component's metric and the rate card its price names count too.
CREATE TEMP TABLE usage_filter_metric ON COMMIT DROP AS
WITH fee_row AS (SELECT sc.subscription_id,
                        sc.price_id,
                        COALESCE(pc.fee_structure, sc.legacy_fee) AS shape,
                        pc.billable_metric_id
                 FROM subscription_component sc
                          LEFT JOIN price_component pc ON pc.id = sc.price_component_id
                 UNION ALL
                 SELECT sa.subscription_id,
                        sa.price_id,
                        COALESCE(a.fee_structure, sa.legacy_fee),
                        NULL::uuid
                 FROM subscription_add_on sa
                          LEFT JOIN add_on a ON a.id = sa.add_on_id)
SELECT DISTINCT bm.id AS metric_id, c.resource_type_id
FROM usage_filter_conversion c
         JOIN fee_row f ON f.subscription_id = c.subscription_id
         LEFT JOIN price p ON p.id = f.price_id
         LEFT JOIN rate_card rc ON rc.tenant_id = c.tenant_id
    AND rc.id = (p.pricing -> 'RateCard' ->> 'rate_card_id')::uuid
         JOIN billable_metric bm ON bm.tenant_id = c.tenant_id
    AND (bm.id = f.billable_metric_id
        OR bm.id = rc.metric_id
        OR f.shape::text LIKE '%' || bm.id || '%');

-- A metric filtered on several properties, a filter left without a resource, or one whose history a
-- resource can't keep (not one filter from the start date, or a start settled after it) fails the
-- migration below: its usage would be billed twice or by no one.
UPDATE billable_metric m
SET attributed_to_type_id = a.resource_type_id
FROM (SELECT metric_id, min(resource_type_id::text)::uuid AS resource_type_id
      FROM usage_filter_metric
      GROUP BY metric_id
      HAVING count(*) = 1) a
WHERE m.id = a.metric_id
  AND m.attributed_to_type_id IS NULL;

DO
$$
    DECLARE
        r RECORD;
    BEGIN
        FOR r IN SELECT metric_id, count(*) AS properties
                 FROM usage_filter_metric
                 GROUP BY metric_id
                 HAVING count(*) > 1
            LOOP
                RAISE EXCEPTION 'usage filters: metric % is filtered on % properties; one '
                    'resource type per metric: convert its subscriptions by hand, then rerun',
                    r.metric_id, r.properties;
            END LOOP;
        FOR r IN SELECT id, usage_filter_property
                 FROM subscription
                 WHERE usage_filter_value IS NOT NULL
                   AND resource_id IS NULL
            LOOP
                RAISE EXCEPTION 'usage filters: subscription % keeps its filter on % without a resource '
                    '(no type keyed by that property, or the code it would get is taken): convert it '
                    'by hand, then rerun',
                    r.id, r.usage_filter_property;
            END LOOP;
        FOR r IN SELECT s.id
                 FROM subscription s
                 WHERE s.status <> 'ABORTED'
                   AND (s.usage_filter_value IS NOT NULL
                     OR EXISTS (SELECT 1 FROM subscription_usage_filter f
                                WHERE f.subscription_id = s.id AND f.property IS NOT NULL))
                   AND NOT (s.usage_filter_value IS NOT NULL
                     AND (SELECT count(*) FROM subscription_usage_filter f WHERE f.subscription_id = s.id) = 1
                     AND EXISTS (SELECT 1 FROM subscription_usage_filter f
                                 WHERE f.subscription_id = s.id
                                   AND f.property = s.usage_filter_property
                                   AND f.value = s.usage_filter_value
                                   AND f.effective_from = s.start_date::timestamp
                                   AND f.effective_until IS NULL)
                     AND COALESCE((SELECT st.at FROM subscription_start_settled st WHERE st.subscription_id = s.id),
                                  s.activated_at,
                                  CASE WHEN s.activation_condition = 'ON_START' THEN s.start_date::timestamp END,
                                  now()::timestamp) <= s.start_date::timestamp)
                 ORDER BY s.id
            LOOP
                RAISE EXCEPTION 'usage filters: subscription % has a filter history other than one filter '
                    'from its start date, or started billing after that date: a resource claims its usage '
                    'from its settled start, so convert it by hand, then rerun', r.id;
            END LOOP;
    END
$$;

-- Every metric a converted subscription bills is attributed to its resource's type, read as billing
-- reads it (`RowFees`): its charge's structure when the price in force today fits it, else its
-- stored fee; and the rate card that price names. Left out, its usage would be billed by no one.
DO
$$
    DECLARE
        r RECORD;
    BEGIN
        FOR r IN WITH fee_row AS (SELECT c.subscription_id,
                                         c.tenant_id,
                                         c.resource_type_id,
                                         sc.price_id,
                                         pc.fee_structure AS charge,
                                         sc.legacy_fee
                                  FROM usage_filter_conversion c
                                           JOIN subscription_component sc ON sc.subscription_id = c.subscription_id
                                           LEFT JOIN price_component pc ON pc.id = sc.price_component_id
                                      AND EXISTS (SELECT 1
                                                  FROM plan_version pv
                                                  WHERE pv.id = pc.plan_version_id
                                                    AND pv.tenant_id = c.tenant_id)
                                  UNION ALL
                                  SELECT c.subscription_id,
                                         c.tenant_id,
                                         c.resource_type_id,
                                         sa.price_id,
                                         a.fee_structure,
                                         sa.legacy_fee
                                  FROM usage_filter_conversion c
                                           JOIN subscription_add_on sa ON sa.subscription_id = c.subscription_id
                                           LEFT JOIN add_on a ON a.id = sa.add_on_id AND a.tenant_id = c.tenant_id),
                      -- The lineage member in force today (R-097): the latest successor in force, else the root.
                      priced AS (SELECT f.*, pr.pricing
                                 FROM fee_row f
                                          LEFT JOIN LATERAL (SELECT p.pricing
                                                             FROM price p
                                                             WHERE p.tenant_id = f.tenant_id
                                                               AND (p.id = f.price_id OR p.supersedes = f.price_id)
                                                               AND (p.effective_from IS NULL
                                                                 OR p.effective_from <= (now() AT TIME ZONE 'UTC')::date)
                                                             ORDER BY p.id = f.price_id, p.effective_from DESC,
                                                                      p.created_at DESC, p.id DESC
                                                             LIMIT 1) pr ON TRUE),
                      billed AS (SELECT subscription_id,
                                        tenant_id,
                                        resource_type_id,
                                        CASE
                                            WHEN charge IS NOT NULL AND pricing ->> 'type' = charge ->> 'type'
                                                THEN charge ->> 'metric_id'
                                            ELSE COALESCE(legacy_fee -> 'Usage' ->> 'metric_id',
                                                          legacy_fee -> 'Capacity' ->> 'metric_id') END AS metric_id
                                 FROM priced
                                 UNION
                                 SELECT p.subscription_id, p.tenant_id, p.resource_type_id, rc.metric_id::text
                                 FROM priced p
                                          JOIN rate_card rc ON rc.tenant_id = p.tenant_id
                                     AND rc.id::text = p.pricing -> 'RateCard' ->> 'rate_card_id')
                 SELECT DISTINCT b.subscription_id, m.id AS metric_id, m.code
                 FROM billed b
                          JOIN billable_metric m ON m.tenant_id = b.tenant_id AND m.id::text = b.metric_id
                 WHERE m.attributed_to_type_id IS DISTINCT FROM b.resource_type_id
                 ORDER BY b.subscription_id, m.id
            LOOP
                RAISE EXCEPTION 'usage filters: subscription % bills metric % (%), which is not attributed to '
                    'the type of its resource (its charges do not name it, or it is attributed to another '
                    'type): its usage would be billed by no one, so convert it by hand, then rerun',
                    r.subscription_id, r.code, r.metric_id;
            END LOOP;
    END
$$;

-- A previous-release pod, still up during the rollout, writing a filter then fails instead of
-- billing it customer-wide. The conversion above left no filter without a resource.
ALTER TABLE subscription
    ADD CONSTRAINT subscription_usage_filter_needs_resource
        CHECK (usage_filter_value IS NULL OR resource_id IS NOT NULL) NOT VALID;
ALTER TABLE subscription
    VALIDATE CONSTRAINT subscription_usage_filter_needs_resource;

-- From when a row a late change wrote bills its metric, past what another subscription of the
-- customer invoiced; its dates and fees stay the change's.
ALTER TABLE subscription_component ADD COLUMN IF NOT EXISTS usage_from TIMESTAMPTZ;
ALTER TABLE subscription_add_on ADD COLUMN IF NOT EXISTS usage_from TIMESTAMPTZ;

-- A draft whose usage changed hands since its lines were computed: refreshed before a hand edit.
-- Beside the invoice, which is at diesel's 64-column limit.
CREATE TABLE invoice_claims_moved
(
    invoice_id UUID PRIMARY KEY REFERENCES invoice (id) ON DELETE CASCADE,
    tenant_id  UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    moved_at   TIMESTAMP NOT NULL
);
CREATE INDEX invoice_claims_moved_tenant_idx ON invoice_claims_moved (tenant_id);

-- Bookkeeping, changing nothing a reader sees: a row's `usage_from` (where a late change's row
-- bills from, its dates and fee unchanged). A subscription's `resource_id` is a change.
CREATE OR REPLACE TRIGGER trg_subscription_component_touch
    AFTER INSERT OR UPDATE OR DELETE ON subscription_component
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity('usage_from');
CREATE OR REPLACE TRIGGER trg_subscription_add_on_touch
    AFTER INSERT OR UPDATE OR DELETE ON subscription_add_on
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity('usage_from');

-- The customers of converted subscriptions: their attribution now reads resources.
INSERT INTO credit_recompute_mark (tenant_id, customer_id)
SELECT DISTINCT tenant_id, customer_id
FROM usage_filter_conversion
ON CONFLICT (tenant_id, customer_id) DO UPDATE SET marked_at = clock_timestamp();
