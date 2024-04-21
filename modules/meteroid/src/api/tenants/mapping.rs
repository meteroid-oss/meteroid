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
    use crate::repo::provider_config::model::RepoProviderConfig;
    use meteroid_grpc::meteroid::api::tenants::v1::TenantBillingConfiguration;
    use secrecy::ExposeSecret;

    pub fn db_to_server(db_model: RepoProviderConfig) -> Option<TenantBillingConfiguration> {
        let api_key = db_model.api_key?.expose_secret().to_string();
        let webhook_secret = db_model.webhook_secret?.expose_secret().to_string();

        Some(TenantBillingConfiguration {
            billing_config_oneof: Some(
                meteroid_grpc::meteroid::api::tenants::v1::tenant_billing_configuration::BillingConfigOneof::Stripe(
                    meteroid_grpc::meteroid::api::tenants::v1::tenant_billing_configuration::Stripe {
                        api_secret: api_key,
                        webhook_secret,
                    },
                ),
            ),
        })
    }
}
