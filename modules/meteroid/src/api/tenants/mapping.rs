pub mod tenants {
    use meteroid_grpc::meteroid::api::tenants::v1::CreateTenantRequest;
    use meteroid_grpc::meteroid::api::tenants::v1::TenantUpdate as GrpcTenantUpdate;
    use meteroid_grpc::meteroid::api::tenants::v1::Tenant;
    use meteroid_grpc::meteroid::api::tenants::v1::TenantEnvironmentEnum as GrpcTenantEnvironmentEnum;
    use meteroid_store::domain;
    use uuid::Uuid;

    pub fn domain_to_server(tenant: domain::Tenant) -> Tenant {
        Tenant {
            id: tenant.id.to_string(),
            name: tenant.name,
            slug: tenant.slug,
            reporting_currency: tenant.currency,
            environment: environment_to_grpc(tenant.environment).into(),
        }
    }

    pub fn update_req_to_domain(req: GrpcTenantUpdate) -> domain::TenantUpdate {
        let environment =
            req.environment.map(|_env| environment_grpc_to_domain(req.environment()));

        environment_grpc_to_domain(req.environment());

        domain::TenantUpdate {
            name: req.name,
            slug: req.slug,
            trade_name: req.trade_name,
            currency: req.reporting_currency,
            environment,
        }
    }

    pub fn create_req_to_domain(req: CreateTenantRequest) -> domain::TenantNew {
        let environment = environment_grpc_to_domain(req.environment());

        domain::TenantNew {
            name: req.name,
            environment,
        }
    }

    pub fn environment_to_grpc(
        env: domain::enums::TenantEnvironmentEnum,
    ) -> GrpcTenantEnvironmentEnum {
        match env {
            domain::enums::TenantEnvironmentEnum::Production => GrpcTenantEnvironmentEnum::Production,
            domain::enums::TenantEnvironmentEnum::Staging => GrpcTenantEnvironmentEnum::Staging,
            domain::enums::TenantEnvironmentEnum::Qa => GrpcTenantEnvironmentEnum::Qa,
            domain::enums::TenantEnvironmentEnum::Development => GrpcTenantEnvironmentEnum::Development,
            domain::enums::TenantEnvironmentEnum::Sandbox => GrpcTenantEnvironmentEnum::Sandbox,
            domain::enums::TenantEnvironmentEnum::Demo => GrpcTenantEnvironmentEnum::Demo,
        }
    }

    pub fn environment_grpc_to_domain(
        env: GrpcTenantEnvironmentEnum,
    ) -> domain::enums::TenantEnvironmentEnum {
        match env {
            GrpcTenantEnvironmentEnum::Production => domain::enums::TenantEnvironmentEnum::Production,
            GrpcTenantEnvironmentEnum::Staging => domain::enums::TenantEnvironmentEnum::Staging,
            GrpcTenantEnvironmentEnum::Qa => domain::enums::TenantEnvironmentEnum::Qa,
            GrpcTenantEnvironmentEnum::Development => domain::enums::TenantEnvironmentEnum::Development,
            GrpcTenantEnvironmentEnum::Sandbox => domain::enums::TenantEnvironmentEnum::Sandbox,
            GrpcTenantEnvironmentEnum::Demo => domain::enums::TenantEnvironmentEnum::Demo,
        }
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
