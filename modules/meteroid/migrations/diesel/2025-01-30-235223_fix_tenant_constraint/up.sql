drop index if exists tenant_slug_key;
create unique index if not exists tenant_org_slug_key on tenant (organization_id, slug);
