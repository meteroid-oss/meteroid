-- MFA / TOTP support
ALTER TABLE "user"
    ADD COLUMN totp_secret text NULL,
    ADD COLUMN totp_enabled boolean NOT NULL DEFAULT false,
    ADD COLUMN totp_recovery_codes text[] NOT NULL DEFAULT '{}',
    ADD COLUMN totp_enrolled_at timestamp(3) NULL;

ALTER TABLE organization
    ADD COLUMN mfa_required boolean NOT NULL DEFAULT false;
