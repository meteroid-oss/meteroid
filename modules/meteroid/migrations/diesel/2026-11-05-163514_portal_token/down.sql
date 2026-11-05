DELETE FROM analytics_dashboard_share WHERE token IS NULL;
ALTER TABLE analytics_dashboard_share ALTER COLUMN token SET NOT NULL;
DROP TABLE IF EXISTS portal_token;
DROP TYPE IF EXISTS "PortalResourceKindEnum";
