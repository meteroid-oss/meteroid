-- The installations an ephemeral license was activated for: each gets one license, never renewed,
-- and activating again before it expires returns it with the same expiry.
CREATE TABLE issued_license_ephemeral_activation
(
    issued_license_id UUID      NOT NULL REFERENCES issued_license (id) ON DELETE CASCADE,
    instance_id       TEXT      NOT NULL,
    activated_at      TIMESTAMP NOT NULL DEFAULT NOW(),
    expires_at        TIMESTAMP NOT NULL,
    PRIMARY KEY (issued_license_id, instance_id)
);
