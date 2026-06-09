CREATE TABLE organization_invite (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id  UUID NOT NULL REFERENCES organization(id),
    invited_email    TEXT NOT NULL,
    invited_by       UUID NOT NULL REFERENCES "user"(id),
    role             "OrganizationUserRole" NOT NULL DEFAULT 'MEMBER'::"OrganizationUserRole",
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at       TIMESTAMPTZ NOT NULL,
    accepted_at      TIMESTAMPTZ,
    revoked_at       TIMESTAMPTZ
);

CREATE UNIQUE INDEX org_invite_pending_email_idx
    ON organization_invite (organization_id, invited_email)
    WHERE accepted_at IS NULL AND revoked_at IS NULL;

ALTER TABLE organization DROP COLUMN IF EXISTS invite_link_hash;
