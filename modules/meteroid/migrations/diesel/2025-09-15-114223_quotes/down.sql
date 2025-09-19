-- Drop triggers and functions
-- DROP TRIGGER IF EXISTS quote_set_number_trigger ON quote;
-- DROP FUNCTION IF EXISTS set_quote_number();
-- DROP FUNCTION IF EXISTS check_quote_expiry();
-- DROP FUNCTION IF EXISTS generate_quote_number(UUID);

-- Drop sequence
-- DROP SEQUENCE IF EXISTS quote_number_seq;

-- Drop tables (in reverse order of creation due to foreign key constraints)
DROP TABLE IF EXISTS quote_activity;
DROP TABLE IF EXISTS quote_signature;
-- DROP TABLE IF EXISTS quote_line_item;
DROP TABLE IF EXISTS quote_component;
DROP TABLE IF EXISTS quote;


-- Drop enum
DROP TYPE IF EXISTS "QuoteStatusEnum";
