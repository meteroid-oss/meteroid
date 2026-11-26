-- A row a late change floored would bill [effective_from, floor) again once the floor is dropped:
-- while one of a live subscription has one, the rollback is refused.
DO
$$
    DECLARE
        r RECORD;
    BEGIN
        SELECT s.id INTO r
        FROM subscription s
        WHERE s.status NOT IN ('CANCELLED', 'COMPLETED', 'ABORTED')
          AND (EXISTS (SELECT 1 FROM subscription_component c
                       WHERE c.subscription_id = s.id AND c.usage_from IS NOT NULL)
            OR EXISTS (SELECT 1 FROM subscription_add_on a
                       WHERE a.subscription_id = s.id AND a.usage_from IS NOT NULL))
        LIMIT 1;
        IF FOUND THEN
            RAISE EXCEPTION 'subscription % bills a late change''s row from its usage floor: rolled back, '
                'it would bill that row''s usage again. End it, or roll forward', r.id;
        END IF;
    END
$$;

-- Subscriptions for a resource get a usage filter back from it (an attributed metric's property,
-- else the canonical key), so none becomes customer-wide. A row's metrics are read as the up
-- migration reads them.
CREATE TEMP TABLE resource_filter ON COMMIT DROP AS
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
SELECT DISTINCT ON (s.id) s.id AS subscription_id, s.tenant_id, s.start_date,
                          COALESCE(bm.attribution_property, k.key) AS property, r.key_value AS value
FROM subscription s
         JOIN resource r ON r.id = s.resource_id
         JOIN resource_type_key k ON k.resource_type_id = r.resource_type_id AND k.position = 0
         LEFT JOIN fee_row f ON f.subscription_id = s.id
         LEFT JOIN price p ON p.id = f.price_id
         LEFT JOIN rate_card rc ON rc.tenant_id = s.tenant_id
    AND rc.id = (p.pricing -> 'RateCard' ->> 'rate_card_id')::uuid
         LEFT JOIN billable_metric bm ON bm.tenant_id = s.tenant_id
    AND bm.attributed_to_type_id = r.resource_type_id
    AND (bm.id = f.billable_metric_id
        OR bm.id = rc.metric_id
        OR f.shape::text LIKE '%' || bm.id || '%')
WHERE s.usage_filter_property IS NULL
ORDER BY s.id, bm.attribution_property NULLS LAST;

-- As the up migration's conversion: no `updated_at` nor activity.
ALTER TABLE subscription DISABLE TRIGGER trg_subscription_touch;
UPDATE subscription s
SET usage_filter_property = f.property,
    usage_filter_value    = f.value,
    usage_filter_since    = f.start_date::timestamp
FROM resource_filter f
WHERE s.id = f.subscription_id;
ALTER TABLE subscription ENABLE TRIGGER trg_subscription_touch;

INSERT INTO subscription_usage_filter (id, tenant_id, subscription_id, property, value, effective_from)
SELECT gen_random_uuid(), tenant_id, subscription_id, property, value, start_date::timestamp
FROM resource_filter;

ALTER TABLE subscription_add_on DROP COLUMN usage_from;
ALTER TABLE subscription_component DROP COLUMN usage_from;
DROP TABLE invoice_claims_moved;

-- Their bookkeeping as before them.
CREATE OR REPLACE TRIGGER trg_subscription_add_on_touch
    AFTER INSERT OR UPDATE OR DELETE ON subscription_add_on
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity();
CREATE OR REPLACE TRIGGER trg_subscription_component_touch
    AFTER INSERT OR UPDATE OR DELETE ON subscription_component
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity();

ALTER TABLE billable_metric
    DROP CONSTRAINT billable_metric_attribution_check,
    DROP COLUMN attributed_to_type_id,
    DROP COLUMN attribution_property;

ALTER TABLE plan
    DROP COLUMN sold_for_resource_type_id;

ALTER TABLE checkout_session
    DROP COLUMN resource_id;

DROP INDEX quote_resource_idx;
ALTER TABLE quote
    DROP CONSTRAINT quote_resource_fkey,
    DROP COLUMN resource_id;

DROP INDEX subscription_resource_idx;
ALTER TABLE subscription
    DROP CONSTRAINT subscription_usage_filter_needs_resource,
    DROP CONSTRAINT subscription_resource_fkey,
    DROP COLUMN resource_id;
