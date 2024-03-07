pub mod tenants {
    use meteroid_grpc::meteroid::api::tenants::v1::Tenant;
    use meteroid_repository::tenants::Tenant as DbTenant;

    pub fn db_to_server(db_model: DbTenant) -> Tenant {
        Tenant {
            id: db_model.id.to_string(),
            name: db_model.name,
            slug: db_model.slug,
            currency: db_model.currency,
        }
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
