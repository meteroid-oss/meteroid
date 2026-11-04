ALTER TABLE tenant
    DROP CONSTRAINT tenant_sandbox_not_live,
    DROP COLUMN email_test_recipients,
    DROP COLUMN email_delivery_mode;
DROP TYPE "EmailDeliveryModeEnum";
