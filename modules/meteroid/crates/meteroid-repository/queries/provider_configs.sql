--: ProviderConfig(webhook_security?, api_security?)

--! get_config_by_provider_and_endpoint () : ProviderConfig
SELECT id, tenant_id, invoicing_provider, enabled, webhook_security, api_security FROM provider_config WHERE tenant_id = :tenant_id AND invoicing_provider = :invoicing_provider AND enabled = TRUE;

--! create_provider_config (webhook_security?, api_security?) : ProviderConfig
INSERT INTO provider_config (id, tenant_id, invoicing_provider, enabled, webhook_security, api_security)
VALUES (:id, :tenant_id, :invoicing_provider, :enabled, :webhook_security, :api_security)
ON CONFLICT (tenant_id, invoicing_provider)
  where enabled = TRUE
  DO UPDATE
  SET
    enabled = EXCLUDED.enabled,
    webhook_security = EXCLUDED.webhook_security,
    api_security = EXCLUDED.api_security
RETURNING id, tenant_id, invoicing_provider, enabled, webhook_security, api_security;
