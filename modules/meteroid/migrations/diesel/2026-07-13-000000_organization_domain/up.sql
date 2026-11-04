-- Organization-level domain ownership, proven via DNS TXT.
--
-- A shared primitive: SSO consumes it to route/enforce logins, and future
-- features (white-label custom domains, email sending) reuse the same proof
-- instead of each re-implementing verification. A domain is "owned" only once
-- `verified_at` is set; an unverified claim is inert.
CREATE TABLE organization_domain (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id     UUID        NOT NULL REFERENCES organization (id) ON DELETE CASCADE,
    -- Lower-cased apex domain (e.g. "acme.com"), no "@".
    domain              TEXT        NOT NULL,
    -- Published at _meteroid-challenge.<domain> (TXT) to prove control. Stable.
    verification_token  TEXT        NOT NULL,
    -- NULL until proven; set at verification time.
    verified_at         TIMESTAMPTZ NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- An org lists a domain at most once.
    CONSTRAINT organization_domain_org_domain_key UNIQUE (organization_id, domain)
);

-- At most one *verified* owner per domain across the whole instance — many orgs
-- may claim (pending), but only the one that proves DNS control owns it.
CREATE UNIQUE INDEX organization_domain_verified_unique
    ON organization_domain (domain)
    WHERE verified_at IS NOT NULL;

-- "Who owns this domain?" — the login-routing lookup.
CREATE INDEX organization_domain_domain_idx ON organization_domain (domain);
