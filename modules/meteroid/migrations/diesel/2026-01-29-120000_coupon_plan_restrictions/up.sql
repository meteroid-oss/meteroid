-- Junction table to restrict coupons to specific plans
-- Empty table means coupon applies to all plans
CREATE TABLE coupon_plan (
    coupon_id UUID NOT NULL REFERENCES coupon(id) ON DELETE CASCADE,
    plan_id UUID NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    PRIMARY KEY (coupon_id, plan_id)
);

CREATE INDEX idx_coupon_plan_plan_id ON coupon_plan(plan_id);
