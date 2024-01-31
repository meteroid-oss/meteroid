--: User()
--: UserWithHash(password_hash?)

--! upsert_user (password_hash?)
INSERT INTO "user" (id, email, password_hash)
VALUES (:id, :email, :password_hash)
ON CONFLICT (id) DO UPDATE
    SET email = EXCLUDED.email
RETURNING id,
    email;

--! get_user_by_id () : User
SELECT
    id,
    email,
    om.role
FROM
    "user"
JOIN organization_member om on "user".id = om.user_id
WHERE
    id = :id;

--! get_user_by_email () : User
SELECT
    id,
    email,
    om.role
FROM
    "user"
        JOIN organization_member om on "user".id = om.user_id
WHERE
    email = :email;

--! get_user_hash_by_email () : UserWithHash
SELECT
    id,
    email,
    password_hash,
    om.role
FROM
    "user"
        JOIN organization_member om on "user".id = om.user_id
WHERE
        email = :email;

--! can_access_tenant ()
SELECT EXISTS (SELECT 1
               FROM organization_member om
                        JOIN tenant t ON om.organization_id = t.organization_id
               WHERE om.user_id = :user_id
                 AND t.id = :tenant_id) AS user_has_access;

--! get_user_role ()
SELECT role
FROM organization_member
WHERE user_id = :user_id
  AND organization_id = :organization_id;

--! get_user_role_by_tenant ()
SELECT role
FROM organization_member om
         JOIN tenant t ON om.organization_id = t.organization_id
WHERE user_id = :user_id
  AND t.id = :tenant_id;

--! get_user_role_by_tenant_slug ()
SELECT role, t.id as tenant_id
FROM organization_member om
         JOIN tenant t ON om.organization_id = t.organization_id
WHERE user_id = :user_id
  AND t.slug = :tenant_slug;

--! get_user_role_oss ()
SELECT role
FROM organization_member
WHERE user_id = :user_id
LIMIT 1;

--! list_users
SELECT
    id,
    email,
    om.role
FROM
    "user"
        JOIN organization_member om on "user".id = om.user_id;

--! exist_users
SELECT EXISTS (SELECT 1 FROM "user") AS user_exists;