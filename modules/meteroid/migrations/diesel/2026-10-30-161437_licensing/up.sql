-- Self-hosted side: this instance's identity and its license. One row at most.
CREATE TABLE instance_license
(
    id                BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    instance_id       UUID      NOT NULL,
    signing_key       TEXT      NOT NULL,
    refresh_token     TEXT,
    license_token     TEXT,
    refreshed_at      TIMESTAMP,
    last_error        TEXT,
    -- A purchase was started from this instance: it may poll for the license until it lands.
    purchase_started_at TIMESTAMP,
    created_at        TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMP NOT NULL DEFAULT NOW(),
    next_refresh_at  TIMESTAMP,
    refresh_failures INT NOT NULL DEFAULT 0,
    -- Meteroid revoked the license with immediate effect; cleared by the next activation.
    revoked_at       TIMESTAMP
);

-- Issuer side: each tenant signs the licenses of its customers with its own Ed25519 key.
CREATE TABLE license_signing_key
(
    id                    UUID PRIMARY KEY,
    tenant_id             UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    kid                   TEXT      NOT NULL,
    public_key_pem        TEXT      NOT NULL,
    -- NULL for Meteroid's own issuer key, which is only ever held in the environment.
    private_key_encrypted TEXT,
    created_at            TIMESTAMP NOT NULL DEFAULT NOW(),
    retired_at            TIMESTAMP
);

CREATE UNIQUE INDEX license_signing_key_tenant_kid_idx ON license_signing_key (tenant_id, kid);
CREATE UNIQUE INDEX license_signing_key_active_idx ON license_signing_key (tenant_id) WHERE retired_at IS NULL;

-- Licenses issued by a tenant to its customers, optionally tied to a subscription.
CREATE TABLE issued_license
(
    id                         UUID PRIMARY KEY,
    tenant_id                  UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id                UUID      NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    subscription_id            UUID REFERENCES subscription (id) ON DELETE SET NULL,
    -- Issued by the subscription reconciler, as opposed to by hand.
    auto_issued                BOOLEAN   NOT NULL DEFAULT FALSE,
    status                     TEXT      NOT NULL CHECK (status IN ('PENDING', 'ACTIVE', 'REVOKED')),
    -- As the installation reports it: a UUID, a machine fingerprint, etc.
    instance_id                TEXT,
    instance_public_key        TEXT,
    -- While PENDING: the activation code, encrypted, so the customer portal can show it.
    activation_code_encrypted  TEXT,
    token_hash                 TEXT      NOT NULL,
    previous_token_hash        TEXT,
    previous_token_valid_until TIMESTAMP,
    validity_days              INT       NOT NULL,
    last_refreshed_at          TIMESTAMP,
    created_at                 TIMESTAMP NOT NULL DEFAULT NOW(),
    revoked_at                 TIMESTAMP,
    -- Revoked to stop at once: the installation drops it on its next refresh, not at its expiry.
    revoked_immediately        BOOLEAN   NOT NULL DEFAULT FALSE,
    -- Issued as a signed offline license: active from issue, never activated nor refreshed.
    offline                    BOOLEAN   NOT NULL DEFAULT FALSE,
    -- Carried by the current refresh token: one presented ahead of it means this database lost
    -- rotations (a failover or restore here), not that the installation is a stale copy.
    token_generation           INT       NOT NULL DEFAULT 0,
    -- The installation process that confirmed it stored the current refresh token.
    holder_session             TEXT,
    -- Another copy of the installation, refused while the holder keeps refreshing.
    contender_session          TEXT,
    contender_since            TIMESTAMP,
    contender_seen_at          TIMESTAMP,
    contender_url              TEXT
);

CREATE UNIQUE INDEX issued_license_token_hash_idx ON issued_license (token_hash);
CREATE INDEX issued_license_previous_token_hash_idx ON issued_license (previous_token_hash);
-- An installation (identified by its key) holds one active license per issuer; activating a
-- new one supersedes the previous.
CREATE UNIQUE INDEX issued_license_instance_key_idx ON issued_license (tenant_id, instance_public_key) WHERE status = 'ACTIVE';
CREATE INDEX issued_license_instance_id_idx ON issued_license (tenant_id, instance_id) WHERE instance_id IS NOT NULL;
-- The reconciler issues at most one live license per subscription.
CREATE UNIQUE INDEX issued_license_auto_subscription_idx ON issued_license (subscription_id) WHERE auto_issued AND status <> 'REVOKED';
CREATE INDEX issued_license_customer_idx ON issued_license (tenant_id, customer_id);
CREATE INDEX issued_license_subscription_idx ON issued_license (subscription_id) WHERE subscription_id IS NOT NULL;

-- The opaque path segment of a tenant's public licensing endpoints, instead of its id.
CREATE TABLE licensing_endpoint
(
    tenant_id  UUID PRIMARY KEY REFERENCES tenant (id) ON DELETE CASCADE,
    slug       TEXT      NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Subscriptions to this plan are issued a license automatically.
ALTER TABLE plan ADD COLUMN issues_license BOOLEAN NOT NULL DEFAULT FALSE;
