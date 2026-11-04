-- Let an invite carry a custom role, mirroring organization_member.role_id.
-- On accept, the new member inherits this role_id; a null role_id keeps the
-- built-in tier from organization_invite.role. Deleting the role clears it.
ALTER TABLE organization_invite
    ADD COLUMN role_id UUID REFERENCES role (id) ON DELETE SET NULL;
