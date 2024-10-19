ALTER TABLE coupon ALTER COLUMN recurring_value SET NOT NULL;

ALTER TABLE coupon DROP CONSTRAINT IF EXISTS positive_recurring_value;
ALTER TABLE coupon DROP CONSTRAINT IF EXISTS positive_redemption_limit;

ALTER TABLE coupon DROP COLUMN IF EXISTS last_redemption_at;
ALTER TABLE coupon DROP COLUMN IF EXISTS archived_at;

DROP INDEX IF EXISTS coupon_tenant_id_code_idx;

drop table if exists applied_coupon;

alter table invoice drop column if exists applied_invoice_ids;
