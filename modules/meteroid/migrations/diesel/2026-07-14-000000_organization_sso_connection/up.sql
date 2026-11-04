-- Organization-level Single Sign-On (SSO) connection.
--
-- One IdP connection per organization. Members whose email domain matches
-- `domains` are routed to the organization's identity provider instead of
-- password login. Distinct from the server-wide social OAuth (Google) sign-in:
-- these credentials are tenant-owned, configured by an org admin, and stored
-- per-organization. OIDC is the only provider for now; `provider` keeps the
-- table forward-compatible with SAML without a schema change.
CREATE TABLE organization_sso_connection (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- One connection per org (v1). Drop the UNIQUE to support multiple IdPs later.
    organization_id     UUID        NOT NULL UNIQUE REFERENCES organization (id) ON DELETE CASCADE,

    -- Protocol discriminator (e.g. "oidc"). Free-form text so adding SAML is
    -- a code-only change.
    provider            TEXT        NOT NULL DEFAULT 'oidc',

    -- Domains this IdP serves (lower-cased, no "@"). A domain only routes /
    -- enforces once it's verified in `organization_domain` — SSO consumes that
    -- shared ownership proof rather than owning verification itself.
    domains             TEXT[]      NOT NULL DEFAULT '{}',

    -- OIDC endpoints. `issuer_url` is kept for display / re-discovery; the three
    -- explicit endpoints are resolved from the issuer's well-known document at
    -- configuration time so the sign-in path never depends on a live discovery call.
    issuer_url          TEXT        NOT NULL,
    authorize_url       TEXT        NOT NULL,
    token_url           TEXT        NOT NULL,
    userinfo_url        TEXT        NOT NULL,

    client_id           TEXT        NOT NULL,
    -- Encrypted at rest (ChaCha20-Poly1305, same crypt key as OAuth verifiers).
    client_secret       TEXT        NOT NULL,
    -- Space-delimited OIDC scopes; "openid" is always required.
    scopes              TEXT        NOT NULL DEFAULT 'openid email profile',

    -- Off by default until an admin verifies the round-trip works.
    enabled             BOOLEAN     NOT NULL DEFAULT false,
    -- When true, password login is refused for `domains` — SSO is mandatory.
    enforced            BOOLEAN     NOT NULL DEFAULT false,
    -- Auto-provision a user + org membership on first successful SSO sign-in.
    jit_provisioning    BOOLEAN     NOT NULL DEFAULT true,

    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NULL,
    -- Admin who last configured the connection; NULL if that user was deleted.
    created_by          UUID        NULL
);

-- Domain → connection lookup on the login path ("which IdP owns this email?").
CREATE INDEX organization_sso_connection_domains_idx
    ON organization_sso_connection USING GIN (domains);
