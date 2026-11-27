DROP TABLE payment_method_expiry_notice;
UPDATE invoicing_entity_email_settings
SET disabled_kinds = array_remove(disabled_kinds, 'payment_method_expiring');
ALTER TABLE invoicing_entity_email_settings
    ALTER COLUMN disabled_kinds SET DEFAULT '{payment_reminder}';
