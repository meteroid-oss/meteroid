--: Organization()
--: OrganizationWithRole()

--! get_organization_by_slug : Organization
SELECT id, name, slug
FROM organization
WHERE slug = :slug;

--! get_organization_by_slug_for_user () : OrganizationWithRole
SELECT o.id, o.name, o.slug, om.role
FROM organization as o
JOIN organization_member AS om ON om.organization_id = o.id
WHERE slug = :slug
AND om.user_id = :user_id;

--! list_organizations_for_user () : OrganizationWithRole
SELECT o.id, o.name, o.slug, om.role
FROM organization AS o
JOIN organization_member AS om ON om.organization_id = o.id
WHERE om.user_id = :user_id;

--! list_organization_members
SELECT
  mem.user_id,
  usr.email AS user_email,
  mem.role AS organization_role
FROM
  organization_member AS mem
JOIN
  "user" AS usr ON mem.user_id = usr.id
WHERE
  mem.organization_id = :organization_id;

--! create_organization : Organization
INSERT INTO organization(id, name, slug)
VALUES (:id, :name, :slug)
RETURNING id, name, slug;


--! create_organization_member 
INSERT INTO organization_member(user_id, organization_id, role)
VALUES (:user_id, :organization_id, :role)
RETURNING user_id, organization_id;

--! instance
SELECT o.id, o.name, o.slug
FROM organization AS o
LIMIT 1;

--! get_invite: (invite_link_hash?)
SELECT invite_link_hash
FROM organization
WHERE id = :organization_id;

--! get_organization_by_invite_hash
SELECT id, name
FROM organization
WHERE invite_link_hash = :invite_hash;


--! set_invite
UPDATE organization
SET invite_link_hash = :invite_link_hash
WHERE id = :organization_id ;