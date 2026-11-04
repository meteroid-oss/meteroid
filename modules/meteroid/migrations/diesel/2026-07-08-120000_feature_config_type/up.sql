-- Add the CONFIG feature type: a static, typed configuration value (number/bool/text/json)
-- resolved synchronously through the entitlement chain, with no metric or usage counter.
ALTER TYPE "FeatureTypeEnum" ADD VALUE IF NOT EXISTS 'CONFIG';
