-- Unify Signals and Alerts: a definition's kind classifies it as a steering
-- signal (risk/opportunity) or a plain operational alert. Default keeps existing
-- alerts operational.
ALTER TABLE alert_definition ADD COLUMN kind TEXT NOT NULL DEFAULT 'operational';
