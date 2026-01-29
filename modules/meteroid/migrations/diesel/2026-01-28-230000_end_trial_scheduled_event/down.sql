-- Note: PostgreSQL doesn't support removing enum values directly
-- The END_TRIAL value will remain in the enum but won't be used after rollback
-- This is safe as long as no END_TRIAL events exist in the database
SELECT 1;
