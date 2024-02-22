--: Tenant()

--! tenants_per_user () : Tenant
SELECT t.id, t.name, t.slug, t.currency
FROM tenant t
JOIN organization o ON t.organization_id = o.id
JOIN organization_member om ON om.organization_id = o.id
JOIN "user" u ON u.id = om.user_id
WHERE u.id = :user_id;

--! get_tenant_by_slug () : Tenant
SELECT t.id, t.name, t.slug, t.currency
FROM tenant AS t
WHERE t.slug = :tenant_slug;

--! get_tenant_by_id () : Tenant
SELECT t.id, t.name, t.slug, t.currency
FROM tenant AS t
WHERE t.id = :tenant_id;

--! create_tenant_for_user : Tenant
INSERT INTO tenant(id, name, slug, organization_id, currency)
VALUES (:id, :name, :slug,
        (SELECT o.id
         FROM organization o
                  JOIN organization_member om ON om.organization_id = o.id
                  JOIN "user" u ON u.id = om.user_id
         WHERE u.id = :user_id LIMIT 1),
        :currency)
RETURNING id, name, slug, currency;

--! create_tenant_for_org : Tenant
INSERT INTO tenant(id, name, slug, organization_id, currency)
VALUES (:id, :name, :slug, :organization_id, :currency)
RETURNING id, name, slug, currency;
