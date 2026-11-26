-- A feature's default is for that feature. Revoked rows are exempt so that pre-existing
-- mismatched rows can still be revoked; migrations/preflight/entitlements_v2.sql (check 11) lists them.
ALTER TABLE entitlement
    ADD CONSTRAINT entitlement_feature_owner_check
        CHECK (entity_type <> 'FEATURE' OR entity_id = feature_id OR revoked_at IS NOT NULL) NOT VALID;
ALTER TABLE entitlement
    VALIDATE CONSTRAINT entitlement_feature_owner_check;
