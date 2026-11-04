-- Postgres does not support removing enum values; a full type rebuild would be required and
-- is not worth the risk for a rollback. Leave the values in place.
SELECT 1;
