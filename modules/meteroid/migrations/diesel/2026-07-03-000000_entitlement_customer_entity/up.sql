-- Customer-level entitlements: a per-account default layer that sits just above the
-- tenant-wide feature default and is overridden by anything a subscription composes.
ALTER TYPE "EntitlementEntityTypeEnum" ADD VALUE IF NOT EXISTS 'CUSTOMER';
