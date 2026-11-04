-- Custom RBAC roles. Built-in Admin/Member remain on organization_member.role;
-- a non-null role_id points at a custom role whose permissions override the
-- built-in Member default. Deleting a role reverts its members to that default.
CREATE TABLE role (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id  UUID NOT NULL REFERENCES organization (id),
    name             TEXT NOT NULL,
    permissions      TEXT[] NOT NULL DEFAULT '{}',
    is_system        BOOLEAN NOT NULL DEFAULT false,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (organization_id, name)
);

ALTER TABLE organization_member
    ADD COLUMN role_id UUID REFERENCES role (id) ON DELETE SET NULL;
