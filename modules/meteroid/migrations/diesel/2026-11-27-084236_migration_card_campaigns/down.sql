ALTER TABLE migration_subscription
    ADD COLUMN payment_method_reminders  BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN payment_method_emails     INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN payment_method_emailed_at TIMESTAMPTZ;
UPDATE migration_subscription ms
SET payment_method_reminders  = m.status IN ('QUEUED', 'EMAILING'),
    payment_method_emails     = m.emails_sent,
    payment_method_emailed_at = m.last_emailed_at
FROM migration_campaign_member m
JOIN migration_campaign c ON c.id = m.campaign_id AND c.is_default
WHERE m.migration_customer_id = ms.migration_customer_id;

DROP INDEX migration_event_campaign_idx;
ALTER TABLE migration_event DROP COLUMN campaign_id;
ALTER TABLE migration_subscription
    DROP COLUMN payment_method_held,
    DROP COLUMN payment_method_waived;
DROP TABLE migration_campaign_send;
DROP TABLE migration_campaign_member;
DROP TABLE migration_campaign;
