-- The expiring-card email is opt-in: off for every invoicing entity until its tenant turns it on.
ALTER TABLE invoicing_entity_email_settings
    ALTER COLUMN disabled_kinds SET DEFAULT '{payment_reminder,payment_method_expiring}';
UPDATE invoicing_entity_email_settings
SET disabled_kinds = array_append(disabled_kinds, 'payment_method_expiring')
WHERE NOT ('payment_method_expiring' = ANY (disabled_kinds));

-- One row per card and expiry the customer was told about, so each is notified once.
CREATE TABLE payment_method_expiry_notice (
    payment_method_id UUID        NOT NULL REFERENCES customer_payment_method (id) ON DELETE CASCADE,
    tenant_id         UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    exp_year          INTEGER     NOT NULL,
    exp_month         INTEGER     NOT NULL,
    notified_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (payment_method_id, exp_year, exp_month)
);
