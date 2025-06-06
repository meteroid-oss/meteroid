pub mod connectors {
    use crate::api::shared::conversions::ProtoConv;
    use itertools::Itertools;
    use meteroid_grpc::meteroid::api::connectors::v1 as server;
    use meteroid_grpc::meteroid::api::connectors::v1::HubspotConnectorData;
    use meteroid_store::domain::connectors as domain;
    use meteroid_store::domain::connectors::{ConnectionMeta, ProviderData};
    use meteroid_store::domain::enums as domain_enum;

    pub fn connector_provider_from_server(
        value: &server::ConnectorProviderEnum,
    ) -> domain_enum::ConnectorProviderEnum {
        match *value {
            server::ConnectorProviderEnum::Stripe => domain_enum::ConnectorProviderEnum::Stripe,
            server::ConnectorProviderEnum::Hubspot => domain_enum::ConnectorProviderEnum::Hubspot,
            server::ConnectorProviderEnum::Pennylane => {
                domain_enum::ConnectorProviderEnum::Pennylane
            }
        }
    }

    pub fn connector_provider_to_server(
        value: &domain_enum::ConnectorProviderEnum,
    ) -> server::ConnectorProviderEnum {
        match *value {
            domain_enum::ConnectorProviderEnum::Stripe => server::ConnectorProviderEnum::Stripe,
            domain_enum::ConnectorProviderEnum::Hubspot => server::ConnectorProviderEnum::Hubspot,
            domain_enum::ConnectorProviderEnum::Pennylane => {
                server::ConnectorProviderEnum::Pennylane
            }
        }
    }

    pub fn connector_type_from_server(
        value: &server::ConnectorTypeEnum,
    ) -> domain_enum::ConnectorTypeEnum {
        match *value {
            server::ConnectorTypeEnum::PaymentProvider => {
                domain_enum::ConnectorTypeEnum::PaymentProvider
            }
            server::ConnectorTypeEnum::Crm => domain_enum::ConnectorTypeEnum::Crm,
            server::ConnectorTypeEnum::Accounting => domain_enum::ConnectorTypeEnum::Accounting,
        }
    }

    pub fn connector_type_to_server(
        value: &domain_enum::ConnectorTypeEnum,
    ) -> server::ConnectorTypeEnum {
        match *value {
            domain_enum::ConnectorTypeEnum::PaymentProvider => {
                server::ConnectorTypeEnum::PaymentProvider
            }
            domain_enum::ConnectorTypeEnum::Crm => server::ConnectorTypeEnum::Crm,
            domain_enum::ConnectorTypeEnum::Accounting => server::ConnectorTypeEnum::Accounting,
        }
    }

    pub fn connector_meta_to_server(value: &domain::ConnectorMeta) -> server::Connector {
        server::Connector {
            id: value.id.as_proto(),
            alias: value.alias.clone(),
            connector_type: connector_type_to_server(&value.connector_type) as i32,
            provider: connector_provider_to_server(&value.provider) as i32,
            data: None,
        }
    }

    pub fn connector_to_server(value: &domain::Connector) -> server::Connector {
        server::Connector {
            id: value.id.as_proto(),
            alias: value.alias.clone(),
            connector_type: connector_type_to_server(&value.connector_type) as i32,
            provider: connector_provider_to_server(&value.provider) as i32,
            data: value.data.as_ref().and_then(|data| match data {
                ProviderData::Stripe(_) => None,
                ProviderData::Hubspot(d) => Some(server::ConnectorData {
                    data: Some(server::connector_data::Data::Hubspot(
                        HubspotConnectorData {
                            auto_sync: d.auto_sync,
                        },
                    )),
                }),
            }),
        }
    }

    pub fn stripe_data_to_domain(value: &server::StripeConnector) -> domain::StripeSensitiveData {
        domain::StripeSensitiveData {
            api_secret_key: value.api_secret_key.clone(),
            webhook_secret: value.webhook_secret.clone(),
        }
    }

    pub fn connection_metadata_to_server(value: &ConnectionMeta) -> server::ConnectionMetadata {
        server::ConnectionMetadata {
            hubspot: value
                .hubspot
                .as_ref()
                .unwrap_or(vec![].as_ref())
                .iter()
                .map(|item| server::ConnectionMetadataItem {
                    connector_id: item.connector_id.as_proto(),
                    external_id: item.external_id.clone(),
                    sync_at: item.sync_at.naive_utc().as_proto(),
                })
                .collect_vec(),
        }
    }
}
