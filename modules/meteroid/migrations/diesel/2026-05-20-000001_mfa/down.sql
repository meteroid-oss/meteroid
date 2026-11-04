ALTER TABLE organization DROP COLUMN mfa_required;

ALTER TABLE "user"
    DROP COLUMN totp_enrolled_at,
    DROP COLUMN totp_recovery_codes,
    DROP COLUMN totp_enabled,
    DROP COLUMN totp_secret;
