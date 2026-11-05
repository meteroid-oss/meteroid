pub mod tenants {
    use crate::tenants::error::TenantApiError;
    use meteroid_grpc::meteroid::api::tenants::v1::CreateTenantRequest;
    use meteroid_grpc::meteroid::api::tenants::v1::EmailDeliveryMode as GrpcEmailDeliveryMode;
    use meteroid_grpc::meteroid::api::tenants::v1::Tenant;
    use meteroid_grpc::meteroid::api::tenants::v1::TenantEnvironmentEnum as GrpcTenantEnvironmentEnum;
    use meteroid_grpc::meteroid::api::tenants::v1::TenantStatusEnum as GrpcTenantStatusEnum;
    use meteroid_grpc::meteroid::api::tenants::v1::TenantUpdate as GrpcTenantUpdate;
    use meteroid_store::domain;

    pub fn domain_to_server(tenant: domain::Tenant) -> Tenant {
        Tenant {
            id: tenant.id.to_string(),
            name: tenant.name,
            slug: tenant.slug,
            reporting_currency: tenant.reporting_currency,
            environment: environment_to_grpc(tenant.environment).into(),
            disable_emails: tenant.email_delivery_mode != domain::enums::EmailDeliveryMode::Live,
            status: status_to_grpc(tenant.status).into(),
            // 0 conveys "grain-derived default" (stored NULL) to the client.
            alert_evaluation_interval_secs: tenant.alert_evaluation_interval_secs.unwrap_or(0),
            email_delivery_mode: delivery_mode_to_grpc(tenant.email_delivery_mode).into(),
            email_test_recipients: tenant.email_test_recipients,
            keep_last_events: tenant.keep_last_events,
            portal_embed_origins: tenant.portal_embed_origins,
        }
    }

    pub fn delivery_mode_to_grpc(mode: domain::enums::EmailDeliveryMode) -> GrpcEmailDeliveryMode {
        match mode {
            domain::enums::EmailDeliveryMode::Live => GrpcEmailDeliveryMode::Live,
            domain::enums::EmailDeliveryMode::Test => GrpcEmailDeliveryMode::Test,
            domain::enums::EmailDeliveryMode::Off => GrpcEmailDeliveryMode::Off,
        }
    }

    /// Strict: an unknown value is refused, never read as the enum's zero (`LIVE`).
    pub fn delivery_mode_from_grpc(
        mode: i32,
    ) -> Result<domain::enums::EmailDeliveryMode, TenantApiError> {
        match GrpcEmailDeliveryMode::try_from(mode) {
            Ok(GrpcEmailDeliveryMode::Live) => Ok(domain::enums::EmailDeliveryMode::Live),
            Ok(GrpcEmailDeliveryMode::Test) => Ok(domain::enums::EmailDeliveryMode::Test),
            Ok(GrpcEmailDeliveryMode::Off) => Ok(domain::enums::EmailDeliveryMode::Off),
            Err(_) => Err(TenantApiError::InvalidArgument(format!(
                "unknown email delivery mode {mode}"
            ))),
        }
    }

    pub fn status_to_grpc(status: domain::enums::TenantStatusEnum) -> GrpcTenantStatusEnum {
        match status {
            domain::enums::TenantStatusEnum::Active => GrpcTenantStatusEnum::Active,
            domain::enums::TenantStatusEnum::Paused => GrpcTenantStatusEnum::Paused,
            domain::enums::TenantStatusEnum::Archived => GrpcTenantStatusEnum::Archived,
            domain::enums::TenantStatusEnum::MarkedForDeletion => {
                GrpcTenantStatusEnum::MarkedForDeletion
            }
            domain::enums::TenantStatusEnum::PendingDeletion => {
                GrpcTenantStatusEnum::PendingDeletion
            }
        }
    }

    pub fn update_req_to_domain(
        req: GrpcTenantUpdate,
    ) -> Result<domain::TenantUpdate, TenantApiError> {
        let environment = req
            .environment
            .map(|_env| environment_grpc_to_domain(req.environment()));
        let email_delivery_mode = req
            .email_delivery_mode
            .map(delivery_mode_from_grpc)
            .transpose()?;

        Ok(domain::TenantUpdate {
            name: req.name,
            slug: req.slug,
            trade_name: req.trade_name,
            reporting_currency: req.reporting_currency,
            environment,
            email_delivery_mode,
            email_test_recipients: req.email_test_recipients.map(|r| r.recipients),
            legacy_disable_emails: req.disable_emails,
            // Omitted => unchanged; 0 => reset to the grain-derived default (NULL);
            // positive => an explicit cadence in seconds.
            alert_evaluation_interval_secs: req
                .alert_evaluation_interval_secs
                .map(|v| if v <= 0 { None } else { Some(v) }),
            keep_last_events: req.keep_last_events,
            portal_embed_origins: req.portal_embed_origins.map(|o| o.origins),
        })
    }

    pub fn create_req_to_domain(
        req: CreateTenantRequest,
    ) -> Result<domain::TenantNew, TenantApiError> {
        let environment = environment_grpc_to_domain(req.environment());
        let email_delivery_mode = match req.email_delivery_mode.map(delivery_mode_from_grpc) {
            Some(mode) => Some(mode?),
            // A client predating the delivery mode asking for emails off.
            None if req.disable_emails == Some(true) => Some(domain::enums::EmailDeliveryMode::Off),
            None => None,
        };

        Ok(domain::TenantNew {
            name: req.name,
            email_delivery_mode,
            environment,
            invoicing_entity: None,
        })
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

#[cfg(test)]
mod tests {
    use super::tenants::delivery_mode_from_grpc;
    use meteroid_store::domain::enums::EmailDeliveryMode;

    #[test]
    fn unknown_delivery_mode_is_refused_not_live() {
        assert_eq!(delivery_mode_from_grpc(1).unwrap(), EmailDeliveryMode::Test);
        assert!(delivery_mode_from_grpc(99).is_err());
    }
}
