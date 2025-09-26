pub mod tenants {
    use meteroid_grpc::meteroid::api::tenants::v1::CreateTenantRequest;
    use meteroid_grpc::meteroid::api::tenants::v1::Tenant;
    use meteroid_grpc::meteroid::api::tenants::v1::TenantEnvironmentEnum as GrpcTenantEnvironmentEnum;
    use meteroid_grpc::meteroid::api::tenants::v1::TenantUpdate as GrpcTenantUpdate;
    use meteroid_store::domain;

    pub fn domain_to_server(tenant: domain::Tenant) -> Tenant {
        Tenant {
            id: tenant.id.to_string(),
            name: tenant.name,
            slug: tenant.slug,
            reporting_currency: tenant.reporting_currency,
            environment: environment_to_grpc(tenant.environment).into(),
            disable_emails: tenant.disable_emails,
        }
    }

    pub fn update_req_to_domain(req: GrpcTenantUpdate) -> domain::TenantUpdate {
        let environment = req
            .environment
            .map(|_env| environment_grpc_to_domain(req.environment()));

        environment_grpc_to_domain(req.environment());

        domain::TenantUpdate {
            name: req.name,
            slug: req.slug,
            trade_name: req.trade_name,
            reporting_currency: req.reporting_currency,
            environment,
            disable_emails: req.disable_emails,
        }
    }

    pub fn create_req_to_domain(req: CreateTenantRequest) -> domain::TenantNew {
        let environment = environment_grpc_to_domain(req.environment());

        domain::TenantNew {
            name: req.name,
            environment,
            disable_emails: req.disable_emails,
        }
    }

    pub fn environment_to_grpc(
        env: domain::enums::TenantEnvironmentEnum,
    ) -> GrpcTenantEnvironmentEnum {
        match env {
            domain::enums::TenantEnvironmentEnum::Production => {
                GrpcTenantEnvironmentEnum::Production
            }
            domain::enums::TenantEnvironmentEnum::Staging => GrpcTenantEnvironmentEnum::Staging,
            domain::enums::TenantEnvironmentEnum::Qa => GrpcTenantEnvironmentEnum::Qa,
            domain::enums::TenantEnvironmentEnum::Development => {
                GrpcTenantEnvironmentEnum::Development
            }
            domain::enums::TenantEnvironmentEnum::Sandbox => GrpcTenantEnvironmentEnum::Sandbox,
            domain::enums::TenantEnvironmentEnum::Demo => GrpcTenantEnvironmentEnum::Demo,
        }
    }

    pub fn environment_grpc_to_domain(
        env: GrpcTenantEnvironmentEnum,
    ) -> domain::enums::TenantEnvironmentEnum {
        match env {
            GrpcTenantEnvironmentEnum::Production => {
                domain::enums::TenantEnvironmentEnum::Production
            }
            GrpcTenantEnvironmentEnum::Staging => domain::enums::TenantEnvironmentEnum::Staging,
            GrpcTenantEnvironmentEnum::Qa => domain::enums::TenantEnvironmentEnum::Qa,
            GrpcTenantEnvironmentEnum::Development => {
                domain::enums::TenantEnvironmentEnum::Development
            }
            GrpcTenantEnvironmentEnum::Sandbox => domain::enums::TenantEnvironmentEnum::Sandbox,
            GrpcTenantEnvironmentEnum::Demo => domain::enums::TenantEnvironmentEnum::Demo,
        }
    }
}
