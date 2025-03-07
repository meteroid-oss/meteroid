pub mod connectors {
    use meteroid_grpc::meteroid::api::connectors::v1 as server;
    use meteroid_store::domain::connectors as domain;
    use meteroid_store::domain::enums as domain_enum;

    pub fn connector_provider_from_server(
        value: &server::ConnectorProviderEnum,
    ) -> domain_enum::ConnectorProviderEnum {
        match value {
            &server::ConnectorProviderEnum::Stripe => domain_enum::ConnectorProviderEnum::Stripe,
        }
    }

    pub fn connector_provider_to_server(
        value: &domain_enum::ConnectorProviderEnum,
    ) -> server::ConnectorProviderEnum {
        match value {
            &domain_enum::ConnectorProviderEnum::Stripe => server::ConnectorProviderEnum::Stripe,
        }
    }

    pub fn connector_type_from_server(
        value: &server::ConnectorTypeEnum,
    ) -> domain_enum::ConnectorTypeEnum {
        match value {
            &server::ConnectorTypeEnum::PaymentProvider => {
                domain_enum::ConnectorTypeEnum::PaymentProvider
            }
        }
    }

    pub fn connector_type_to_server(
        value: &domain_enum::ConnectorTypeEnum,
    ) -> server::ConnectorTypeEnum {
        match value {
            &domain_enum::ConnectorTypeEnum::PaymentProvider => {
                server::ConnectorTypeEnum::PaymentProvider
            }
        }
    }

    pub fn connector_meta_to_server(value: &domain::ConnectorMeta) -> server::Connector {
        server::Connector {
            id: value.id.as_proto(),
            alias: value.alias.clone(),
            connector_type: connector_type_to_server(&value.connector_type) as i32,
            provider: connector_provider_to_server(&value.provider) as i32,
        }
    }

    pub fn stripe_data_to_domain(value: &server::StripeConnector) -> domain::StripeSensitiveData {
        domain::StripeSensitiveData {
            api_secret_key: value.api_secret_key.clone(),
            webhook_secret: value.webhook_secret.clone(),
        }
    }
}
