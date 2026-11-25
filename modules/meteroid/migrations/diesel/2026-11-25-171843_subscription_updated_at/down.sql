DROP TRIGGER IF EXISTS trg_subscription_ending_touch ON subscription_ending;
DROP TRIGGER IF EXISTS trg_subscription_change_touch ON subscription_change;
DROP TRIGGER IF EXISTS trg_slot_transaction_touch ON slot_transaction;
DROP TRIGGER IF EXISTS trg_applied_coupon_touch ON applied_coupon;
DROP TRIGGER IF EXISTS trg_subscription_add_on_touch ON subscription_add_on;
DROP TRIGGER IF EXISTS trg_subscription_component_touch ON subscription_component;
DROP TRIGGER IF EXISTS trg_subscription_touch ON subscription;
DROP TRIGGER IF EXISTS trg_subscription_activity_forget ON subscription;

DROP FUNCTION IF EXISTS subscription_note_activity();
DROP FUNCTION IF EXISTS subscription_touch();
DROP FUNCTION IF EXISTS subscription_activity_forget();

-- Its flush trigger goes with it.
DROP TABLE IF EXISTS subscription_activity_pending;
DROP FUNCTION IF EXISTS subscription_activity_flush();

DROP TABLE IF EXISTS subscription_activity;
ALTER TABLE subscription DROP COLUMN updated_at;
