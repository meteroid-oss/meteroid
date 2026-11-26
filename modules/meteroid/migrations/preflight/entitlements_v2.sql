-- Read-only preflight for entitlements v2 (ADR 0011): the migrations 2026-11-26-011437_entitlements_v2,
-- 2026-11-26-011852_entitlements_v2_billing and 2026-11-26-012219_entitlement_feature_owner, then the
-- rollout. Run it with psql against production (a replica is fine: SELECTs only) right BEFORE
-- deploying, and again right AFTER for checks 6 and 15. It needs the facts release (2.3.0 or later: `subscription_start_settled`); on an
-- older install upgrade to 2.3.x first. Columns a schema may lack are read through to_jsonb, so it
-- runs on the schema before the deploy and on the one after.
--
-- The first statement says where the database stands. The second lists, per check and tenant, the
-- rows found (count, up to 20 ids, up to 5 details). Verdicts:
--   BLOCKS        the migration raises on these rows: the deploy fails at startup until they are fixed.
--   BLOCKS LATER  passes today, raises if the deploy runs after the date in the detail.
--   CHANGES       migrates, but behaves differently from deploy on: check with the tenant.
--   REVIEW        migrates; worth a look (data the API would refuse, sandbox-only facts the ADR assumes).
--   CLEANUP       migrates; rows the API's checks would never have let in.
--   AFTER DEPLOY  only rows after the deploy: what the rollout window may have mis-billed.
-- No BLOCKS row means the migrations will not stop on data.
--
--  1 BLOCKS   (011437) A metered entitlement value whose threshold, warning threshold, grace or reset
--             interval is not a whole number (0 to 999999999), or whose `thresholds` is not a list.
--             Fix: correct the value JSON (the detail shows the terms) or delete the row.
--  2 BLOCKS   (011852) A metric priced by subscriptions filtered on several properties: one resource
--             type per metric. Fix: move those subscriptions to one property, or end the ones on the
--             other property and recreate them as resource subscriptions after the deploy. The id is
--             the metric's. Pricing is matched by any mention of the metric id in the charge
--             structures, so it may list a metric the migration accepts, never miss one.
--  3 BLOCKS   (011852) A usage filter value without a property. Fix: set the property or clear the value.
--  4 BLOCKS   (011852) A subscription (not aborted) whose filter history is not one open filter from
--             its start date, or whose billing started after its start date. Fix: by hand, per the
--             detail: end it and recreate it for a resource after the deploy, or clear its filter.
--  5 BLOCKS LATER  As 4, for a filtered subscription not activated yet whose start date is ahead: the
--             migration raises if it runs after that date and the subscription is still not active.
--  6 REVIEW   Every subscription the migration converts to a resource. ADR §1.2 fact 1 holds that
--             only development and sandbox tenants have any: a PRODUCTION row contradicts it, ask the
--             merchant before deploying. After the deploy, a row marked "not converted" was written
--             with a filter by a previous-release pod during the rollout: convert it by hand.
--  7 REVIEW   A usage filter property a resource type code cannot spell (anything but lowercase
--             letters, digits and `_`, or starting with a digit or `_`, or clashing): the code the
--             migration gives it. The type's key stays the property, so events keep matching.
--  8 CHANGES  A trialing subscription (TRIAL_ACTIVE) whose plan version names a trialing plan with a
--             published version: from deploy, its entitlements resolve that plan's version (§3.9).
--             Compare the two plans' entitlements; nothing to fix unless the tenant objects.
--  9 CHANGES  A metered entitlement with `group_by`: the limit applies to the customer as a whole
--             from deploy (ADR §1.2 fact 2 holds there is none). Recreate it per resource after the deploy.
-- 10 CHANGES  A limited add-on row whose limit terms (reset, overage, grace, thresholds) differ from
--             the policy its feature gets: it keeps its limit, under the feature's terms (§1.2 fact 3).
-- 11 BLOCKS   (012219) An open feature default (entity FEATURE) naming another feature than its own:
--             validating `entitlement_feature_owner_check` fails on it (FOLLOWUPS item 8). Fix: delete
--             the row before the deploy (every row is open before it; the constraint lets revoked ones by).
-- 12 CLEANUP  An open entitlement row whose owner is missing or another tenant's, or whose feature is
--             another tenant's (FOLLOWUPS item 8). Delete it before the deploy, or revoke it after.
-- 13 REVIEW   A quote draft or template naming a feature to create (no feature id): the editor of this
--             release drops those entitlements when it loads it. Tell its author, or create the feature.
-- 14 REVIEW   A converted resource whose key value the API refuses (empty, surrounding spaces, control
--             characters, over 255 bytes): it migrates, but cannot be recreated as is, and live limits
--             cannot group its usage (EE).
-- 15 AFTER DEPLOY  An invoice finalized within an hour of the billing migration for a customer with a
--             subscription for a resource created by the new release: a previous-release pod, still up,
--             bills such a subscription customer-wide and does not exclude its usage from the
--             customer's other subscriptions. Compare it with a fresh preview; credit the difference.
--             `run_on` is in the database's time zone and `finalized_at` in UTC: off UTC, shift it.
-- 16 CHANGES  (011437) An entitlement value a live subscription resolves differently from deploy on, for
--             a feature an add-on of it has a row on (§2.2). v1 took the highest of Plan < AddOn <
--             PlanVersion < Subscription rows, so a plan version's or the subscription's row hid the
--             add-ons, an add-on's replaced the plan's, add-on instances were not counted and the
--             last add-on config value won. From deploy an add-on row adds to the plan's value
--             (multi-instance, times its instances) or raises it (an override add-on, whose lower
--             value no longer wins: numbers get `number_higher_is_better`). The detail gives the
--             feature and both values; a seat add-on's adds the seats held, not computed. A customer
--             row decides both before and after, so its features are left out. Run before the deploy.
--             Fix: confirm the new values with the tenant; to keep an old one, after the deploy grant
--             it to the customer as a Set to row, or change the add-on's or the plan's row.
-- 17 BLOCKS   (011852) A metric a converted subscription bills (its charge's structure when the price
--             in force today fits it, else its stored fee, as billing reads them) that the charges of
--             no subscription filtered on its property name: the migration would not attribute it to
--             the subscription's type, its usage billed by no one, and raises. Rate cards are new in
--             this deploy (their metric is their charge's). Fix: correct the row's charge or price, or
--             end the subscription and recreate it for a resource after the deploy.

SELECT (SELECT max(version) FROM __diesel_schema_migrations)                                 AS latest_migration,
       EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = '20261116171538')    AS facts_release_applied,
       EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = '20261126011437')    AS entitlements_v2_applied,
       EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = '20261126011852')    AS entitlements_v2_billing_applied,
       -- Dropped by 011437 by this name: missing before it, the migration fails.
       EXISTS (SELECT 1
               FROM pg_constraint
               WHERE conrelid = 'entitlement'::regclass
                 AND conname = 'entitlement_feature_id_entity_id_entity_type_key')           AS old_unique_constraint_present;

-- The checks.
WITH RECURSIVE applied AS (SELECT EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = '20261126011852') AS billing,
                        (SELECT run_on FROM __diesel_schema_migrations WHERE version = '20261126011852') AS billing_run_on),
     -- 011437's DO block, verbatim.
     c1 AS (SELECT 1 AS check_no,
                   e.tenant_id,
                   e.id,
                   jsonb_strip_nulls(jsonb_build_object(
                           'thresholds', e.value -> 'thresholds',
                           'warning_threshold_pct', e.value -> 'warning_threshold_pct',
                           'grace_period_pct', e.value -> 'overage_behavior' -> 'grace_period_pct',
                           'interval', e.value -> 'reset_period' -> 'interval'))::text AS detail
            FROM entitlement e
            WHERE e.value ->> 'type' = 'metered'
              AND (jsonb_typeof(COALESCE(e.value -> 'thresholds', 'null')) NOT IN ('array', 'null')
                OR EXISTS (SELECT 1
                           FROM jsonb_array_elements_text(CASE jsonb_typeof(e.value -> 'thresholds')
                                                              WHEN 'array' THEN e.value -> 'thresholds'
                                                              ELSE '[]' END) t
                           WHERE t !~ '^[0-9]{1,9}$')
                OR e.value ->> 'warning_threshold_pct' !~ '^[0-9]{1,9}$'
                OR e.value -> 'overage_behavior' ->> 'grace_period_pct' !~ '^[0-9]{1,9}$'
                OR e.value -> 'reset_period' ->> 'interval' !~ '^[0-9]{1,9}$')),
     -- 011852's usage_filter_conversion: every filtered subscription, whatever its status.
     conv AS (SELECT s.id                    AS subscription_id,
                     s.tenant_id,
                     s.customer_id,
                     s.usage_filter_property AS property,
                     s.usage_filter_value    AS key_value,
                     s.status::text          AS status,
                     to_jsonb(s) ->> 'resource_id' AS resource_id
              FROM subscription s
              WHERE s.usage_filter_value IS NOT NULL
                AND s.usage_filter_property IS NOT NULL),
     -- 011852's fee rows: a charge's structure is its component's or add-on's (after the deploy),
     -- its product's (before), else the row's own; all of them are searched for the metric id.
     fee_row AS (SELECT sc.subscription_id,
                        concat_ws(' ', (to_jsonb(pc) -> 'fee_structure')::text, pp.fee_structure::text,
                                  sc.legacy_fee::text) AS shape,
                        pc.billable_metric_id
                 FROM subscription_component sc
                          LEFT JOIN price_component pc ON pc.id = sc.price_component_id
                          LEFT JOIN product pp ON pp.id = pc.product_id
                 WHERE sc.subscription_id IN (SELECT subscription_id FROM conv)
                 UNION ALL
                 SELECT sa.subscription_id,
                        concat_ws(' ', (to_jsonb(a) -> 'fee_structure')::text, ap.fee_structure::text,
                                  sa.legacy_fee::text),
                        NULL::uuid
                 FROM subscription_add_on sa
                          LEFT JOIN add_on a ON a.id = sa.add_on_id
                          LEFT JOIN product ap ON ap.id = a.product_id
                 WHERE sa.subscription_id IN (SELECT subscription_id FROM conv)),
     metric_property AS (SELECT DISTINCT bm.tenant_id, bm.id AS metric_id, bm.code, c.property
                         FROM conv c
                                  JOIN fee_row f ON f.subscription_id = c.subscription_id
                                  JOIN billable_metric bm ON bm.tenant_id = c.tenant_id
                             AND (bm.id = f.billable_metric_id OR f.shape LIKE '%' || bm.id || '%')),
     c2 AS (SELECT 2 AS check_no,
                   tenant_id,
                   metric_id                                                          AS id,
                   format('metric %s filtered on: %s', code, string_agg(property, ', ' ORDER BY property)) AS detail
            FROM metric_property
            GROUP BY tenant_id, metric_id, code
            HAVING count(*) > 1),
     c3 AS (SELECT 3 AS check_no, s.tenant_id, s.id, format('value %L, no property', s.usage_filter_value) AS detail
            FROM subscription s
            WHERE s.usage_filter_value IS NOT NULL
              AND s.usage_filter_property IS NULL),
     -- 011852's third loop, its condition split into its terms.
     history AS (SELECT s.tenant_id,
                        s.id,
                        s.status::text                                                       AS status,
                        s.usage_filter_value IS NOT NULL                                     AS has_filter,
                        (SELECT count(*) FROM subscription_usage_filter f WHERE f.subscription_id = s.id) AS versions,
                        EXISTS (SELECT 1
                                FROM subscription_usage_filter f
                                WHERE f.subscription_id = s.id
                                  AND f.property = s.usage_filter_property
                                  AND f.value = s.usage_filter_value
                                  AND f.effective_from = s.start_date::timestamp
                                  AND f.effective_until IS NULL)                             AS from_start,
                        COALESCE((SELECT st.at FROM subscription_start_settled st WHERE st.subscription_id = s.id),
                                 s.activated_at,
                                 CASE WHEN s.activation_condition::text = 'ON_START' THEN s.start_date::timestamp END,
                                 now()::timestamp)                                           AS started,
                        (SELECT st.at FROM subscription_start_settled st WHERE st.subscription_id = s.id) IS NULL
                            AND s.activated_at IS NULL
                            AND s.activation_condition::text <> 'ON_START'                   AS not_started,
                        s.start_date::timestamp                                              AS start_at
                 FROM subscription s
                 WHERE s.status::text <> 'ABORTED'
                   AND (s.usage_filter_value IS NOT NULL
                     OR EXISTS (SELECT 1
                                FROM subscription_usage_filter f
                                WHERE f.subscription_id = s.id
                                  AND f.property IS NOT NULL))),
     c4 AS (SELECT 4 AS check_no,
                   tenant_id,
                   id,
                   concat_ws('; ',
                             status,
                             CASE WHEN NOT has_filter THEN 'its filter was removed' END,
                             CASE WHEN versions <> 1 THEN format('%s filter versions', versions) END,
                             CASE WHEN has_filter AND NOT from_start THEN 'its filter is not one open version from its start date' END,
                             CASE WHEN started > start_at AND NOT not_started
                                      THEN format('billing started %s, after its start date %s', started, start_at) END,
                             CASE WHEN started > start_at AND not_started
                                      THEN format('not activated, start date %s passed', start_at) END) AS detail
            FROM history
            WHERE NOT (has_filter AND versions = 1 AND from_start AND started <= start_at)),
     c5 AS (SELECT 5 AS check_no, tenant_id, id, format('%s; not activated: raises if migrated after %s', status, start_at) AS detail
            FROM history
            WHERE has_filter
              AND versions = 1
              AND from_start
              AND not_started
              AND started <= start_at),
     c6 AS (SELECT 6 AS check_no,
                   c.tenant_id,
                   c.subscription_id                                                   AS id,
                   format('%s=%s, %s%s', c.property, c.key_value, c.status,
                          CASE WHEN a.billing AND c.resource_id IS NULL THEN ', NOT CONVERTED: convert by hand' ELSE '' END) AS detail
            FROM conv c
                     CROSS JOIN applied a),
     -- 011852's usage_filter_type, for a tenant without types (none exist before the deploy).
     slug AS (SELECT p.tenant_id,
                     p.property,
                     left(CASE
                              WHEN p.s = '' THEN 'resource'
                              WHEN p.s ~ '^[a-z]' THEN p.s
                              ELSE 'r_' || p.s END, 60) AS base
              FROM (SELECT DISTINCT tenant_id,
                                    usage_filter_property AS property,
                                    btrim(regexp_replace(lower(usage_filter_property), '[^a-z0-9_]+', '_', 'g'),
                                          '_')        AS s
                    FROM subscription
                    WHERE usage_filter_property IS NOT NULL) p),
     coded AS (SELECT tenant_id,
                      property,
                      CASE
                          WHEN row_number() OVER w = 1 THEN base
                          ELSE base || '_' || row_number() OVER w END AS code
               FROM slug
               WINDOW w AS (PARTITION BY tenant_id, base ORDER BY property = base DESC, property)),
     c7 AS (SELECT 7 AS check_no,
                   tenant_id,
                   NULL::uuid                                                          AS id,
                   format('property %L: type code %s, key %L', property, code, property) AS detail
            FROM coded
            WHERE code <> property),
     c8 AS (SELECT 8 AS check_no,
                   s.tenant_id,
                   s.id,
                   format('subscribed to %s, resolves %s (version %s)', sp.name, tp.name, tp.active_version_id) AS detail
            FROM subscription s
                     JOIN plan_version pv ON pv.id = s.plan_version_id
                     JOIN plan sp ON sp.id = pv.plan_id
                     JOIN plan tp ON tp.id = pv.trialing_plan_id AND tp.tenant_id = s.tenant_id
            WHERE s.status::text = 'TRIAL_ACTIVE'
              AND tp.active_version_id IS NOT NULL
              AND tp.active_version_id <> s.plan_version_id),
     c9 AS (SELECT 9 AS check_no,
                   e.tenant_id,
                   e.id,
                   format('%s %s, group_by %s', e.entity_type, e.entity_id, e.value ->> 'group_by') AS detail
            FROM entitlement e
            WHERE e.value ->> 'type' = 'metered'
              AND e.value ->> 'group_by' IS NOT NULL
              AND (to_jsonb(e) ->> 'revoked_at') IS NULL),
     -- 011437's legacy_policy and policy vote, for the rows its DO block accepts.
     legacy AS (SELECT e.id,
                       e.tenant_id,
                       e.feature_id,
                       e.entity_type::text                 AS entity_type,
                       e.entity_id,
                       e.created_at,
                       e.value ->> 'limit' IS NOT NULL     AS limited,
                       jsonb_strip_nulls(jsonb_build_object(
                               'resets', CASE e.value -> 'reset_period' ->> 'type'
                                             WHEN 'billing_cycle' THEN 'BILLING_CYCLE'
                                             WHEN 'calendar' THEN 'CALENDAR'
                                             WHEN 'fixed_window' THEN 'FIXED'
                                             WHEN 'sliding_window' THEN 'SLIDING'
                                             ELSE 'NEVER' END,
                               'period', CASE
                                             WHEN e.value -> 'reset_period' ->> 'type' IN ('calendar', 'fixed_window', 'sliding_window')
                                                 THEN jsonb_build_object(
                                                     'unit', COALESCE(e.value -> 'reset_period' ->> 'unit', 'day'),
                                                     'interval', COALESCE((e.value -> 'reset_period' ->> 'interval')::int, 1)) END,
                               'at_limit', CASE
                                               WHEN e.value -> 'overage_behavior' ->> 'type' = 'allow' THEN 'ALLOW_AND_BILL'
                                               ELSE 'DENY' END,
                               'grace_pct', CASE
                                                WHEN e.value -> 'overage_behavior' ->> 'type' = 'allow' THEN 0
                                                ELSE COALESCE((e.value -> 'overage_behavior' ->> 'grace_period_pct')::int, 0) END,
                               'thresholds', (SELECT COALESCE(jsonb_agg(t ORDER BY t), '[]')
                                              FROM (SELECT t::int AS t
                                                    FROM jsonb_array_elements_text(CASE jsonb_typeof(e.value -> 'thresholds')
                                                                                       WHEN 'array' THEN e.value -> 'thresholds'
                                                                                       ELSE '[]' END) t
                                                    UNION
                                                    SELECT (e.value ->> 'warning_threshold_pct')::int
                                                    WHERE e.value ->> 'warning_threshold_pct' IS NOT NULL
                                                    ORDER BY 1
                                                    LIMIT 10) ts))) AS policy
                FROM entitlement e
                WHERE e.value ->> 'type' = 'metered'
                  AND e.id NOT IN (SELECT id FROM c1)),
     votes AS (SELECT feature_id,
                      policy,
                      count(*) FILTER (WHERE limited)  AS limited,
                      count(*)                         AS n,
                      bool_or(entity_type = 'FEATURE') AS is_default,
                      min(created_at)                  AS first_at
               FROM legacy
               GROUP BY feature_id, policy),
     feature_policy AS (SELECT DISTINCT ON (feature_id) feature_id, policy
                        FROM votes
                        ORDER BY feature_id, limited DESC, n DESC, is_default DESC, first_at),
     c10 AS (SELECT 10 AS check_no,
                    l.tenant_id,
                    l.id,
                    format('add-on %s: its terms %s become the feature''s %s', l.entity_id, l.policy, fp.policy) AS detail
             FROM legacy l
                      JOIN feature_policy fp ON fp.feature_id = l.feature_id
             WHERE l.entity_type = 'ADD_ON'
               AND l.limited
               AND l.policy <> fp.policy),
     -- FOLLOWUPS item 8, open rows only: the constraint lets revoked ones through.
     c11 AS (SELECT 11 AS check_no,
                    e.tenant_id,
                    e.id,
                    format('default of feature %s naming feature %s', e.feature_id, e.entity_id) AS detail
             FROM entitlement e
             WHERE e.entity_type::text = 'FEATURE'
               AND e.entity_id <> e.feature_id
               AND (to_jsonb(e) ->> 'revoked_at') IS NULL),
     c12 AS (SELECT 12 AS check_no,
                    e.tenant_id,
                    e.id,
                    concat_ws('; ',
                              e.entity_type::text || ' ' || e.entity_id,
                              CASE WHEN COALESCE(f.tenant_id, c.tenant_id, p.tenant_id, pv.tenant_id, a.tenant_id,
                                                 s.tenant_id, q.tenant_id) IS DISTINCT FROM e.tenant_id
                                       THEN 'owner missing or another tenant''s' END,
                              CASE WHEN ff.tenant_id IS DISTINCT FROM e.tenant_id
                                       THEN 'feature missing or another tenant''s' END) AS detail
             FROM entitlement e
                      LEFT JOIN feature ff ON ff.id = e.feature_id
                      LEFT JOIN feature f ON e.entity_type::text = 'FEATURE' AND f.id = e.entity_id
                      LEFT JOIN customer c ON e.entity_type::text = 'CUSTOMER' AND c.id = e.entity_id
                      LEFT JOIN plan p ON e.entity_type::text = 'PLAN' AND p.id = e.entity_id
                      LEFT JOIN plan_version pv ON e.entity_type::text = 'PLAN_VERSION' AND pv.id = e.entity_id
                      LEFT JOIN add_on a ON e.entity_type::text = 'ADD_ON' AND a.id = e.entity_id
                      LEFT JOIN subscription s ON e.entity_type::text = 'SUBSCRIPTION' AND s.id = e.entity_id
                      LEFT JOIN quote q ON e.entity_type::text = 'QUOTE' AND q.id = e.entity_id
             WHERE (to_jsonb(e) ->> 'revoked_at') IS NULL
               AND (COALESCE(f.tenant_id, c.tenant_id, p.tenant_id, pv.tenant_id, a.tenant_id, s.tenant_id,
                             q.tenant_id) IS DISTINCT FROM e.tenant_id
                 OR ff.tenant_id IS DISTINCT FROM e.tenant_id)),
     c13 AS (SELECT 13 AS check_no,
                    d.tenant_id,
                    d.id,
                    format('quote draft%s: %s', COALESCE(' of quote ' || d.quote_id, ''),
                           jsonb_path_query_array(d.payload,
                                                  '$.pendingEntitlements[*] ? (exists(@.featureName) && !(exists(@.featureId))).featureName')) AS detail
             FROM quote_draft d
             WHERE jsonb_path_exists(d.payload,
                                     '$.pendingEntitlements[*] ? (exists(@.featureName) && !(exists(@.featureId)))')
             UNION ALL
             SELECT 13,
                    t.tenant_id,
                    t.id,
                    format('quote template %L: %s', t.name,
                           jsonb_path_query_array(t.payload,
                                                  '$.pendingEntitlements[*] ? (exists(@.featureName) && !(exists(@.featureId))).featureName'))
             FROM quote_template t
             WHERE t.archived_at IS NULL
               AND jsonb_path_exists(t.payload,
                                     '$.pendingEntitlements[*] ? (exists(@.featureName) && !(exists(@.featureId)))')),
     c14 AS (SELECT 14 AS check_no,
                    tenant_id,
                    subscription_id AS id,
                    format('%s=%L: %s', property, left(key_value, 64),
                           concat_ws(', ',
                                     CASE WHEN key_value !~ '\S' THEN 'blank' END,
                                     CASE WHEN key_value ~ '^\s|\s$' THEN 'surrounding spaces' END,
                                     CASE WHEN key_value ~ '[\x01-\x1f\x7f-\x9f]' THEN 'control characters' END,
                                     CASE WHEN octet_length(key_value) > 255 THEN 'over 255 bytes' END)) AS detail
             FROM conv
             WHERE key_value !~ '\S'
                OR key_value ~ '^\s|\s$'
                OR key_value ~ '[\x01-\x1f\x7f-\x9f]'
                OR octet_length(key_value) > 255),
     c15 AS (SELECT 15 AS check_no,
                    i.tenant_id,
                    i.id,
                    format('invoice %s finalized %s, %s after the migration', i.invoice_number, i.finalized_at,
                           i.finalized_at - a.billing_run_on) AS detail
             FROM invoice i
                      CROSS JOIN applied a
             WHERE a.billing
               AND i.finalized_at >= a.billing_run_on
               AND i.finalized_at < a.billing_run_on + interval '1 hour'
               AND EXISTS (SELECT 1
                           FROM subscription s
                           WHERE s.tenant_id = i.tenant_id
                             AND s.customer_id = i.customer_id
                             AND s.usage_filter_value IS NULL
                             AND (to_jsonb(s) ->> 'resource_id') IS NOT NULL)),
     -- v1's resolution of each live subscription (`resolve`, read day and add-ons in force as
     -- `in_force_sql`), beside the deploy's fold, on the features an add-on of it has a row on.
     live_sub AS (SELECT s.tenant_id,
                         s.id                                                                  AS subscription_id,
                         s.customer_id,
                         s.plan_version_id,
                         pv.plan_id,
                         LEAST(current_date, COALESCE(s.current_period_end - 1, current_date)) AS read_day
                  FROM subscription s
                           JOIN plan_version pv ON pv.id = s.plan_version_id
                  WHERE s.status::text IN ('ACTIVE', 'TRIAL_ACTIVE')),
     live_add_on AS (SELECT l.tenant_id,
                            l.subscription_id,
                            sa.add_on_id,
                            sum(sa.quantity)                                                     AS instances,
                            bool_or(a.max_instances_per_subscription IS NULL
                                OR a.max_instances_per_subscription > 1)                         AS multi,
                            bool_or(COALESCE(NULLIF(to_jsonb(a) -> 'fee_structure', 'null'),
                                             to_jsonb(ap) -> 'fee_structure') ->> 'type' = 'Slot') AS seat
                     FROM live_sub l
                              JOIN subscription_add_on sa ON sa.subscription_id = l.subscription_id
                              JOIN add_on a ON a.id = sa.add_on_id
                              LEFT JOIN product ap ON ap.id = a.product_id
                     WHERE sa.effective_from <= l.read_day
                       AND (sa.effective_to IS NULL OR sa.effective_to > l.read_day)
                     GROUP BY l.tenant_id, l.subscription_id, sa.add_on_id),
     -- An add-on row's effect as 011437's legacy_effect gives it.
     add_on_row AS (SELECT la.tenant_id,
                           la.subscription_id,
                           e.feature_id,
                           e.id,
                           e.created_at,
                           e.value,
                           'ADD_ON'::text AS entity,
                           la.instances,
                           la.multi,
                           la.seat,
                           COALESCE(to_jsonb(e) ->> 'effect',
                                    CASE
                                        WHEN e.value ->> 'type' = 'config'
                                            AND COALESCE(e.value -> 'value' ->> 'kind', '') NOT IN ('number', 'boolean')
                                            THEN 'SET'
                                        WHEN e.mode::text = 'STACK' THEN 'ADD'
                                        ELSE 'RAISE_TO' END) AS effect
                    FROM live_add_on la
                             JOIN entitlement e ON e.tenant_id = la.tenant_id
                        AND e.entity_type::text = 'ADD_ON'
                        AND e.entity_id = la.add_on_id
                    WHERE (to_jsonb(e) ->> 'revoked_at') IS NULL),
     chain_row AS (SELECT *
                   FROM add_on_row
                   UNION ALL
                   SELECT p.tenant_id,
                          p.subscription_id,
                          e.feature_id,
                          e.id,
                          e.created_at,
                          e.value,
                          e.entity_type::text,
                          1,
                          false,
                          false,
                          'SET'
                   FROM (SELECT DISTINCT tenant_id, subscription_id, feature_id FROM add_on_row) p
                            JOIN live_sub l ON l.subscription_id = p.subscription_id
                            JOIN entitlement e ON e.tenant_id = p.tenant_id
                       AND e.feature_id = p.feature_id
                       AND e.entity_id = CASE e.entity_type::text
                                             WHEN 'FEATURE' THEN e.feature_id
                                             WHEN 'PLAN' THEN l.plan_id
                                             WHEN 'PLAN_VERSION' THEN l.plan_version_id
                                             WHEN 'SUBSCRIPTION' THEN l.subscription_id
                                             WHEN 'CUSTOMER' THEN l.customer_id END
                   WHERE (to_jsonb(e) ->> 'revoked_at') IS NULL),
     -- A row's value as both read it; one whose value doesn't fit its feature counts in neither.
     chain_value AS (SELECT r.*,
                            k.code,
                            k.kind,
                            k.higher_is_better,
                            CASE
                                WHEN k.kind = 'limit' THEN NOT COALESCE((r.value ->> 'enabled')::boolean, true)
                                WHEN k.kind IN ('bool', 'config_bool')
                                    THEN NOT COALESCE((r.value ->> 'enabled')::boolean,
                                                      (r.value -> 'value' ->> 'value')::boolean, false)
                                ELSE false END                                                  AS is_off,
                            k.kind = 'limit' AND r.value ->> 'limit' IS NULL                    AS unlimited,
                            CASE
                                WHEN k.kind = 'limit' THEN (r.value ->> 'limit')::numeric
                                WHEN k.kind = 'number' THEN (r.value -> 'value' ->> 'value')::numeric END AS n,
                            CASE WHEN k.kind = 'unordered' THEN r.value -> 'value' -> 'value' END AS config
                     FROM chain_row r
                              JOIN (SELECT f.id,
                                           f.code,
                                           CASE
                                               WHEN f.feature_type::text = 'METERED' THEN 'limit'
                                               WHEN f.feature_type::text = 'BOOLEAN' THEN 'bool'
                                               WHEN f.config_value_type::text = 'BOOLEAN' THEN 'config_bool'
                                               WHEN f.config_value_type::text = 'NUMBER' THEN 'number'
                                               ELSE 'unordered' END                                     AS kind,
                                           COALESCE((to_jsonb(f) ->> 'number_higher_is_better')::boolean, true) AS higher_is_better
                                    FROM feature f
                                    WHERE f.status::text = 'ACTIVE') k ON k.id = r.feature_id
                     WHERE CASE k.kind
                               WHEN 'limit' THEN r.value ->> 'type' = 'metered'
                               WHEN 'bool' THEN r.value ->> 'type' = 'boolean'
                               WHEN 'number' THEN r.value -> 'value' ->> 'kind' = 'number'
                               WHEN 'unordered' THEN r.value ->> 'type' = 'config'
                               ELSE r.value ->> 'type' = 'boolean' OR r.value -> 'value' ->> 'kind' = 'boolean' END),
     -- v1's add-on bucket: in creation order, a multi-instance add-on's limit adds, another's raises.
     v1_seq AS (SELECT subscription_id,
                       feature_id,
                       multi,
                       COALESCE(n, 0)                                                                    AS n,
                       row_number() OVER (PARTITION BY subscription_id, feature_id ORDER BY created_at, id) AS k
                FROM chain_value
                WHERE entity = 'ADD_ON'
                  AND kind = 'limit'),
     v1_sum (subscription_id, feature_id, k, n) AS (SELECT subscription_id, feature_id, k, n
                                                   FROM v1_seq
                                                   WHERE k = 1
                                                   UNION ALL
                                                   SELECT q.subscription_id,
                                                          q.feature_id,
                                                          q.k,
                                                          CASE WHEN q.multi THEN s.n + q.n ELSE GREATEST(s.n, q.n) END
                                                   FROM v1_sum s
                                                            JOIN v1_seq q ON q.subscription_id = s.subscription_id
                                                       AND q.feature_id = s.feature_id
                                                       AND q.k = s.k + 1),
     v1_sum_last AS (SELECT DISTINCT ON (subscription_id, feature_id) subscription_id, feature_id, n
                     FROM v1_sum
                     ORDER BY subscription_id, feature_id, k DESC),
     -- v1: the subscription's row, else the plan version's, hides the add-ons.
     v1_top AS (SELECT DISTINCT ON (subscription_id, feature_id) subscription_id, feature_id, is_off, unlimited, n, config
                FROM chain_value
                WHERE entity IN ('SUBSCRIPTION', 'PLAN_VERSION')
                ORDER BY subscription_id, feature_id, entity = 'SUBSCRIPTION' DESC, created_at DESC, id DESC),
     newest_add_on AS (SELECT DISTINCT ON (subscription_id, feature_id) subscription_id, feature_id, is_off, n, config
                       FROM chain_value
                       WHERE entity = 'ADD_ON'
                       ORDER BY subscription_id, feature_id, created_at DESC, id DESC),
     -- v2: the highest tier's Set to below the add-ons, then the add-ons' rows.
     v2_base AS (SELECT DISTINCT ON (subscription_id, feature_id) subscription_id, feature_id, is_off, unlimited, n, config
                 FROM chain_value
                 WHERE entity IN ('FEATURE', 'PLAN', 'PLAN_VERSION', 'SUBSCRIPTION')
                 ORDER BY subscription_id, feature_id,
                          array_position(ARRAY ['FEATURE', 'PLAN', 'PLAN_VERSION', 'SUBSCRIPTION'], entity) DESC,
                          created_at, id),
     oldest_add_on_set AS (SELECT DISTINCT ON (subscription_id, feature_id) subscription_id, feature_id, config
                           FROM chain_value
                           WHERE entity = 'ADD_ON'
                             AND effect = 'SET'
                           ORDER BY subscription_id, feature_id, created_at, id),
     add_on_terms AS (SELECT tenant_id,
                             subscription_id,
                             feature_id,
                             code,
                             kind,
                             higher_is_better,
                             bool_or(entity = 'CUSTOMER')                                     AS has_customer,
                             bool_and(is_off) FILTER (WHERE entity = 'ADD_ON')                   AS all_off,
                             bool_or(unlimited) FILTER (WHERE entity = 'ADD_ON')              AS any_unlimited,
                             bool_or(entity = 'ADD_ON' AND seat AND effect = 'ADD')           AS seat_add,
                             bool_or(grants AND effect = 'ADD')                               AS adds,
                             bool_or(grants AND effect = 'ADD' AND unlimited)                 AS adds_unlimited,
                             sum(n * instances) FILTER (WHERE grants AND effect = 'ADD' AND NOT unlimited) AS added,
                             bool_or(grants AND effect = 'RAISE_TO')                          AS raises,
                             bool_or(grants AND effect = 'RAISE_TO' AND unlimited)            AS raises_unlimited,
                             max(n) FILTER (WHERE grants AND effect = 'RAISE_TO' AND NOT unlimited) AS raised,
                             min(n) FILTER (WHERE grants AND effect <> 'SET')                 AS lowest,
                             bool_or(grants AND effect <> 'SET')                              AS any_on
                      FROM (SELECT *, entity = 'ADD_ON' AND NOT is_off AND instances > 0 AS grants FROM chain_value) v
                      GROUP BY tenant_id, subscription_id, feature_id, code, kind, higher_is_better
                      HAVING bool_or(entity = 'ADD_ON')),
     resolved AS (SELECT t.tenant_id,
                         t.subscription_id,
                         t.code,
                         t.kind,
                         t.has_customer,
                         t.seat_add,
                         CASE
                             WHEN v1.subscription_id IS NOT NULL THEN v1.is_off
                             WHEN t.kind IN ('limit', 'bool') THEN t.all_off
                             ELSE na.is_off END                                                     AS v1_off,
                         CASE
                             WHEN v1.subscription_id IS NOT NULL THEN v1.unlimited
                             ELSE t.kind = 'limit' AND t.any_unlimited END                        AS v1_unlimited,
                         CASE
                             WHEN v1.subscription_id IS NOT NULL THEN v1.n
                             WHEN t.kind = 'limit' THEN fl.n
                             ELSE na.n END                                                       AS v1_n,
                         CASE WHEN v1.subscription_id IS NOT NULL THEN v1.config ELSE na.config END   AS v1_json,
                         CASE
                             WHEN t.kind IN ('bool', 'config_bool') THEN NOT (COALESCE(NOT b.is_off, false) OR t.any_on)
                             WHEN t.kind = 'limit' AND t.raises THEN false
                             WHEN t.kind = 'limit' THEN COALESCE(a.is_off, true)
                             ELSE false END                                                      AS v2_off,
                         CASE
                             WHEN t.kind <> 'limit' THEN false
                             WHEN t.raises THEN t.raises_unlimited OR COALESCE(NOT a.is_off AND a.unlimited, false)
                             ELSE COALESCE(a.unlimited, false) END                               AS v2_unlimited,
                         CASE
                             WHEN t.kind = 'number' AND NOT t.higher_is_better THEN LEAST(b.n, t.lowest)
                             WHEN t.kind IN ('limit', 'number') AND t.raises
                                 THEN GREATEST(CASE WHEN NOT COALESCE(a.is_off, true) AND NOT a.unlimited THEN a.n END,
                                               t.raised)
                             WHEN t.kind IN ('limit', 'number') THEN a.n END                     AS v2_n,
                         CASE WHEN os.subscription_id IS NOT NULL THEN os.config ELSE b.config END    AS v2_json
                  FROM add_on_terms t
                           LEFT JOIN v1_top v1 ON v1.subscription_id = t.subscription_id AND v1.feature_id = t.feature_id
                           LEFT JOIN v1_sum_last fl ON fl.subscription_id = t.subscription_id AND fl.feature_id = t.feature_id
                           LEFT JOIN newest_add_on na ON na.subscription_id = t.subscription_id AND na.feature_id = t.feature_id
                           LEFT JOIN v2_base b ON b.subscription_id = t.subscription_id AND b.feature_id = t.feature_id
                           LEFT JOIN oldest_add_on_set os
                                     ON os.subscription_id = t.subscription_id AND os.feature_id = t.feature_id
                           -- The value after the Add rows: from 0 over nothing or off, unlimited absorbing.
                           CROSS JOIN LATERAL (SELECT CASE WHEN t.adds THEN false ELSE b.is_off END AS is_off,
                                                      CASE
                                                          WHEN t.adds THEN COALESCE(NOT b.is_off AND b.unlimited, false)
                                                              OR t.adds_unlimited
                                                          ELSE b.unlimited END                AS unlimited,
                                                      CASE
                                                          WHEN t.adds THEN COALESCE(CASE WHEN NOT b.is_off AND NOT b.unlimited THEN b.n END, 0)
                                                              + COALESCE(t.added, 0)
                                                          ELSE b.n END                        AS n) a),
     shown AS (SELECT r.*,
                      CASE
                          WHEN kind = 'unordered' THEN COALESCE(v1_json::text, 'none')
                          WHEN v1_off THEN 'off'
                          WHEN kind IN ('bool', 'config_bool') THEN 'on'
                          WHEN v1_unlimited THEN 'unlimited'
                          ELSE COALESCE(v1_n::text, 'none') END                                AS v1_text,
                      CASE
                          WHEN seat_add THEN 'not computed (a seat add-on adds per seat held)'
                          WHEN kind = 'unordered' THEN COALESCE(v2_json::text, 'none')
                          WHEN v2_off THEN 'off'
                          WHEN kind IN ('bool', 'config_bool') THEN 'on'
                          WHEN v2_unlimited THEN 'unlimited'
                          ELSE COALESCE(v2_n::text, 'none') END                                AS v2_text,
                      CASE kind
                          WHEN 'unordered' THEN v1_json IS DISTINCT FROM v2_json
                          WHEN 'number' THEN v1_n IS DISTINCT FROM v2_n
                          WHEN 'limit' THEN (v1_off, v1_unlimited AND NOT v1_off,
                                             CASE WHEN NOT v1_off AND NOT v1_unlimited THEN v1_n END)
                              IS DISTINCT FROM (v2_off, v2_unlimited AND NOT v2_off,
                                                CASE WHEN NOT v2_off AND NOT v2_unlimited THEN v2_n END)
                          ELSE v1_off IS DISTINCT FROM v2_off END                              AS differs
               FROM resolved r),
     c16 AS (SELECT 16 AS check_no,
                    tenant_id,
                    subscription_id                                                  AS id,
                    format('feature %s: %s before, %s after', code, v1_text, v2_text) AS detail
             FROM shown
             WHERE NOT has_customer
               AND (seat_add OR differs)),
     -- 011852's last DO block, before the deploy: the metric each row of a converted subscription
     -- bills (`RowFees`), and the metrics its charges name for the conversion (`usage_filter_metric`,
     -- a charge holding its product's structure from 2026-11-25-093418_charge_owns_shape).
     conv_fee AS (SELECT c.tenant_id,
                         c.subscription_id,
                         c.property,
                         'component ' || sc.name AS row_name,
                         sc.price_id,
                         sc.legacy_fee,
                         pc.billable_metric_id,
                         CASE
                             WHEN to_jsonb(pc) ? 'fee_structure' THEN NULLIF(to_jsonb(pc) -> 'fee_structure', 'null')
                             WHEN EXISTS (SELECT 1 FROM plan_component_price l WHERE l.plan_component_id = pc.id)
                                 THEN to_jsonb(pp) -> 'fee_structure' END AS charge
                  FROM conv c
                           JOIN subscription_component sc ON sc.subscription_id = c.subscription_id
                           LEFT JOIN price_component pc ON pc.id = sc.price_component_id
                           LEFT JOIN product pp ON pp.id = pc.product_id
                  UNION ALL
                  SELECT c.tenant_id,
                         c.subscription_id,
                         c.property,
                         'add-on ' || sa.name,
                         sa.price_id,
                         sa.legacy_fee,
                         NULL::uuid,
                         CASE
                             WHEN to_jsonb(a) ? 'fee_structure' THEN NULLIF(to_jsonb(a) -> 'fee_structure', 'null')
                             ELSE to_jsonb(ap) -> 'fee_structure' END
                  FROM conv c
                           JOIN subscription_add_on sa ON sa.subscription_id = c.subscription_id
                           LEFT JOIN add_on a ON a.id = sa.add_on_id
                           LEFT JOIN product ap ON ap.id = a.product_id),
     conv_billed AS (SELECT f.tenant_id,
                            f.subscription_id,
                            f.property,
                            f.row_name,
                            CASE
                                WHEN f.charge IS NOT NULL AND pr.pricing ->> 'type' = f.charge ->> 'type'
                                    THEN f.charge ->> 'metric_id'
                                ELSE COALESCE(f.legacy_fee -> 'Usage' ->> 'metric_id',
                                              f.legacy_fee -> 'Capacity' ->> 'metric_id') END AS metric_id
                     FROM conv_fee f
                              LEFT JOIN LATERAL (SELECT p.pricing
                                                 FROM price p
                                                 WHERE p.tenant_id = f.tenant_id
                                                   AND (p.id = f.price_id OR p.supersedes = f.price_id)
                                                   AND (p.effective_from IS NULL
                                                     OR p.effective_from <= (now() AT TIME ZONE 'UTC')::date)
                                                 ORDER BY p.id = f.price_id, p.effective_from DESC, p.created_at DESC,
                                                          p.id DESC
                                                 LIMIT 1) pr ON TRUE),
     conv_named AS (SELECT DISTINCT f.tenant_id, bm.id AS metric_id, f.property
                    FROM conv_fee f
                             JOIN billable_metric bm ON bm.tenant_id = f.tenant_id
                        AND (bm.id = f.billable_metric_id
                            OR COALESCE(f.charge, f.legacy_fee)::text LIKE '%' || bm.id || '%')),
     c17 AS (SELECT DISTINCT 17 AS check_no,
                             b.tenant_id,
                             b.subscription_id AS id,
                             format('%s bills metric %s; its charges name it for: %s', b.row_name, bm.code,
                                    COALESCE((SELECT string_agg(n.property, ', ' ORDER BY n.property)
                                              FROM conv_named n
                                              WHERE n.tenant_id = b.tenant_id
                                                AND n.metric_id = bm.id), 'no filtered subscription')) AS detail
             FROM conv_billed b
                      JOIN billable_metric bm ON bm.tenant_id = b.tenant_id AND bm.id::text = b.metric_id
             WHERE NOT EXISTS (SELECT 1
                               FROM conv_named n
                               WHERE n.tenant_id = b.tenant_id
                                 AND n.metric_id = bm.id
                                 AND n.property = b.property)),
     checks AS (SELECT * FROM c1
                UNION ALL SELECT * FROM c2
                UNION ALL SELECT * FROM c3
                UNION ALL SELECT * FROM c4
                UNION ALL SELECT * FROM c5
                UNION ALL SELECT * FROM c6
                UNION ALL SELECT * FROM c7
                UNION ALL SELECT * FROM c8
                UNION ALL SELECT * FROM c9
                UNION ALL SELECT * FROM c10
                UNION ALL SELECT * FROM c11
                UNION ALL SELECT * FROM c12
                UNION ALL SELECT * FROM c13
                UNION ALL SELECT * FROM c14
                UNION ALL SELECT * FROM c15
                UNION ALL SELECT * FROM c16
                UNION ALL SELECT * FROM c17),
     verdicts (check_no, verdict) AS (VALUES (1, 'BLOCKS'), (2, 'BLOCKS'), (3, 'BLOCKS'), (4, 'BLOCKS'),
                                             (5, 'BLOCKS LATER'), (6, 'REVIEW'), (7, 'REVIEW'), (8, 'CHANGES'),
                                             (9, 'CHANGES'), (10, 'CHANGES'), (11, 'BLOCKS'), (12, 'CLEANUP'),
                                             (13, 'REVIEW'), (14, 'REVIEW'), (15, 'AFTER DEPLOY'), (16, 'CHANGES'),
                                             (17, 'BLOCKS'))
SELECT c.check_no,
       v.verdict,
       c.tenant_id,
       t.name                                                                AS tenant,
       t.environment::text                                                   AS environment,
       count(*)                                                              AS found,
       (array_agg(c.id ORDER BY c.id) FILTER (WHERE c.id IS NOT NULL))[1:20] AS sample_ids,
       (array_agg(COALESCE(c.detail, '') ORDER BY c.id, c.detail))[1:5]      AS sample_details
FROM checks c
         JOIN verdicts v ON v.check_no = c.check_no
         LEFT JOIN tenant t ON t.id = c.tenant_id
GROUP BY c.check_no, v.verdict, c.tenant_id, t.name, t.environment
ORDER BY c.check_no, t.environment::text = 'PRODUCTION' DESC, t.name, c.tenant_id;
