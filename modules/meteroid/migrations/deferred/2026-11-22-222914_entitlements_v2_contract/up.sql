-- Entitlements v2 contract (ADR 0011 §7): drops what only v1 reads. Kept out of ../diesel until
-- the release whose code no longer reads it; move it there with a new version.
DO
$$
    DECLARE
        r RECORD;
    BEGIN
        FOR r IN SELECT tenant_id, count(*) AS n FROM entitlement WHERE value ? 'group_by' GROUP BY 1
            LOOP
                RAISE NOTICE 'group_by: tenant % has % rows with a group_by, stripped', r.tenant_id, r.n;
            END LOOP;
    END
$$;
UPDATE entitlement
SET value = value - 'group_by'
WHERE value ? 'group_by';

ALTER TABLE feature
    DROP COLUMN feature_type;
DROP TYPE "FeatureTypeEnum";
