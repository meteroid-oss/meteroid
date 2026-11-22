ALTER TABLE organization_domain
    DROP COLUMN check_failing_since,
    DROP COLUMN last_checked_at;

DROP TABLE password_reset_token;

ALTER TABLE "user" DROP COLUMN session_epoch;

DROP INDEX user_email_lower_idx;
