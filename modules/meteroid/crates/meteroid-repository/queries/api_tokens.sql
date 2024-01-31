--: ApiToken()

--! list_api_tokens () : ApiToken
SELECT id, tenant_id, name, hint, created_at, created_by FROM api_token WHERE tenant_id = :tenant_id;

--! create_api_token () : ApiToken
INSERT INTO api_token (id, name, hint, hash, tenant_id, created_by) 
VALUES (:id, :name, :hint, :hash, :tenant_id, :created_by)
RETURNING id, tenant_id, name, hint, created_at, created_by;

--! get_api_token_by_id ()
SELECT hash, tenant_id FROM api_token WHERE id = :id;
