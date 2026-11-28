DROP TRIGGER IF EXISTS trg_applied_coupon_analytics_tombstone ON applied_coupon;
DROP TRIGGER IF EXISTS trg_coupon_analytics_tombstone ON coupon;
DROP FUNCTION IF EXISTS analytics_tombstone_applied_coupon();
DROP FUNCTION IF EXISTS analytics_tombstone_coupon();
DROP TABLE IF EXISTS analytics_tombstone;
DROP TRIGGER IF EXISTS trg_quote_store_committed_at ON quote;
ALTER TABLE quote DROP COLUMN IF EXISTS store_committed_at;
DROP TRIGGER IF EXISTS trg_applied_coupon_touch ON applied_coupon;
CREATE TRIGGER trg_applied_coupon_touch
    AFTER INSERT OR UPDATE OR DELETE ON applied_coupon
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity('applied_count', 'applied_amount',
                                                             'last_applied_at');
DROP TRIGGER IF EXISTS trg_applied_coupon_store_committed_at ON applied_coupon;
ALTER TABLE applied_coupon DROP COLUMN IF EXISTS store_committed_at;
DROP TRIGGER IF EXISTS trg_coupon_store_committed_at ON coupon;
ALTER TABLE coupon DROP COLUMN IF EXISTS store_committed_at;
DROP TABLE IF EXISTS sql_api_key;
DROP TABLE IF EXISTS sql_query_run;
DROP TABLE IF EXISTS sql_query_schedule;
DROP TABLE IF EXISTS sql_saved_query;
-- 'SQL_RESULT' stays in "StoredObjectKindEnum": Postgres cannot drop an enum value.
