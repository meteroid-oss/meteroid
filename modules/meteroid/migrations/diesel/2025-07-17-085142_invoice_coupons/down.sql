
ALTER TABLE invoice
  DROP COLUMN IF EXISTS coupons,
  DROP COLUMN IF EXISTS discount,
  DROP COLUMN IF EXISTS purchase_order;

ALTER TABLE invoice ADD COLUMN applied_coupon_ids UUID[] NOT NULL DEFAULT '{}';
