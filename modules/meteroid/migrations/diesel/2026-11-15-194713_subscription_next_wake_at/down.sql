DROP INDEX IF EXISTS subscription_next_wake_at_idx;
ALTER TABLE subscription DROP COLUMN next_wake_at;
