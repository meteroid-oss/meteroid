-- The shapes come back; group_by stays lost, feature_type is read back from kind.
CREATE TYPE "FeatureTypeEnum" AS ENUM ('BOOLEAN', 'METERED', 'CONFIG');
ALTER TABLE feature
    ADD COLUMN feature_type "FeatureTypeEnum" NOT NULL DEFAULT 'BOOLEAN';
UPDATE feature
SET feature_type = CASE
                       WHEN kind = 'LIMIT' THEN 'METERED'
                       WHEN kind = 'VALUE' OR config_value_type IS NOT NULL THEN 'CONFIG'
                       ELSE 'BOOLEAN' END::"FeatureTypeEnum";
