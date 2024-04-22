pub mod tenants {
    use meteroid_grpc::meteroid::api::tenants::v1::CreateTenantRequest;
    use meteroid_grpc::meteroid::api::tenants::v1::Tenant;
    use meteroid_store::domain;
    use uuid::Uuid;

    pub fn domain_to_server(tenant: domain::Tenant) -> Tenant {
        Tenant {
            id: tenant.id.to_string(),
            name: tenant.name,
            slug: tenant.slug,
            currency: tenant.currency,
        }
    }

    pub fn create_req_to_domain(req: CreateTenantRequest, user_id: Uuid) -> domain::TenantNew {
        domain::TenantNew::ForUser(domain::UserTenantNew {
            name: req.name,
            currency: req.currency,
            slug: req.slug,
            user_id,
            environment: None, // todo add to the api
        })
    }
}

pub mod provider_configs {
    use crate::api::tenants::error::TenantApiError;
    use meteroid_grpc::meteroid::api::tenants::v1::tenant_billing_configuration::BillingConfigOneof;
    use meteroid_grpc::meteroid::api::tenants::v1::{
        ConfigureTenantBillingRequest, TenantBillingConfiguration,
    };
    use meteroid_store::domain::configs::{
        ApiSecurity, ProviderConfig, ProviderConfigNew, WebhookSecurity,
    };
    use meteroid_store::domain::enums::InvoicingProviderEnum;
    use uuid::Uuid;

    pub fn domain_to_server(db_model: ProviderConfig) -> TenantBillingConfiguration {
        TenantBillingConfiguration {
            billing_config_oneof: Some(BillingConfigOneof::Stripe(
                meteroid_grpc::meteroid::api::tenants::v1::tenant_billing_configuration::Stripe {
                    api_secret: db_model.api_security.api_key,
                    webhook_secret: db_model.webhook_security.secret,
                },
            )),
        }
    }

    pub fn create_req_server_to_domain(
        req: ConfigureTenantBillingRequest,
        tenant_id: Uuid,
    ) -> Result<ProviderConfigNew, TenantApiError> {
        let billing_config = req
            .billing_config
            .clone()
            .ok_or(TenantApiError::MissingArgument(
                "billing_config".to_string(),
            ))?
            .billing_config_oneof
            .ok_or(TenantApiError::MissingArgument(
                "billing_config_oneof".to_string(),
            ))?;

        let cfg = match billing_config {
            BillingConfigOneof::Stripe(stripe) => ProviderConfigNew {
                tenant_id,
                invoicing_provider: InvoicingProviderEnum::Stripe,
                enabled: true,
                webhook_security: WebhookSecurity {
                    secret: stripe.webhook_secret,
                },
                api_security: ApiSecurity {
                    api_key: stripe.api_secret,
                },
            },
        };

        Ok(cfg)
    }
}
