-- Where a workspace's customer emails go: LIVE = customers; TEST = org members and the
-- allowlist only (everyone else is logged as skipped); OFF = nobody, logged as skipped.
-- Replaces disable_emails, kept one release for pods still reading it (written = mode <> LIVE).
CREATE TYPE "EmailDeliveryModeEnum" AS ENUM ('LIVE', 'TEST', 'OFF');

ALTER TABLE tenant
    ADD COLUMN email_delivery_mode "EmailDeliveryModeEnum" NOT NULL DEFAULT 'TEST',
    -- Addresses, or '@domain' for a whole domain, that TEST mode also delivers to.
    ADD COLUMN email_test_recipients TEXT[] NOT NULL DEFAULT '{}',
    -- A sandbox holds seeded data and never emails customers.
    ADD CONSTRAINT tenant_sandbox_not_live
        CHECK (environment <> 'SANDBOX' OR email_delivery_mode <> 'LIVE');

-- Emails off stay off. Emails on stay on (a sandbox goes to TEST), even outside production:
-- only production can be switched to LIVE from now on (enforced in the store).
UPDATE tenant SET email_delivery_mode = CASE
    WHEN disable_emails THEN 'OFF'
    WHEN environment = 'SANDBOX' THEN 'TEST'
    ELSE 'LIVE'
END::"EmailDeliveryModeEnum";
-- Pods still reading disable_emails during the rollout must hold whatever is not LIVE.
UPDATE tenant SET disable_emails = (email_delivery_mode <> 'LIVE');
