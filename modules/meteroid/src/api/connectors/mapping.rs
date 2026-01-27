pub mod connectors {
    use crate::api::shared::conversions::ProtoConv;
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

    /// Converts a domain connector provider to the server/API representation.
    /// Returns None for Mock connectors, which should not be exposed via API.
    pub fn connector_provider_to_server(
        value: &domain_enum::ConnectorProviderEnum,
    ) -> Option<server::ConnectorProviderEnum> {
        match *value {
            domain_enum::ConnectorProviderEnum::Stripe => {
                Some(server::ConnectorProviderEnum::Stripe)
            }
            domain_enum::ConnectorProviderEnum::Hubspot => {
                Some(server::ConnectorProviderEnum::Hubspot)
            }
            domain_enum::ConnectorProviderEnum::Pennylane => {
                Some(server::ConnectorProviderEnum::Pennylane)
            }
            domain_enum::ConnectorProviderEnum::Mock => {
                // Mock connector is for testing only - should never be returned via API
                log::warn!(
                    "Attempted to expose Mock connector via API - this should not happen in production"
                );
                None
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

    /// Converts a domain ConnectorMeta to server representation.
    /// Returns None for Mock connectors, which should not be exposed via API.
    pub fn connector_meta_to_server(value: &domain::ConnectorMeta) -> Option<server::Connector> {
        let provider = connector_provider_to_server(&value.provider)?;
        Some(server::Connector {
            id: value.id.as_proto(),
            alias: value.alias.clone(),
            connector_type: connector_type_to_server(&value.connector_type) as i32,
            provider: provider as i32,
            data: None,
        })
    }

    /// Converts a domain Connector to server representation.
    /// Returns None for Mock connectors, which should not be exposed via API.
    pub fn connector_to_server(value: &domain::Connector) -> Option<server::Connector> {
        let provider = connector_provider_to_server(&value.provider)?;
        Some(server::Connector {
            id: value.id.as_proto(),
            alias: value.alias.clone(),
            connector_type: connector_type_to_server(&value.connector_type) as i32,
            provider: provider as i32,
            data: value.data.as_ref().and_then(|data| match data {
                ProviderData::Stripe(_) => None,
                ProviderData::Hubspot(d) => Some(server::ConnectorData {
                    data: Some(server::connector_data::Data::Hubspot(
                        HubspotConnectorData {
                            external_company_id: d.external_company_id.clone(),
                            auto_sync: d.auto_sync,
                        },
                    )),
                }),
                ProviderData::Pennylane(d) => Some(server::ConnectorData {
                    data: Some(server::connector_data::Data::Pennylane(
                        server::PennylaneConnectorData {
                            external_company_id: d.external_company_id.clone(),
                        },
                    )),
                }),
                // Mock is for testing only, no data exposed in API
                ProviderData::Mock(_) => None,
            }),
        })
    }

    pub fn stripe_data_to_domain(value: &server::StripeConnector) -> domain::StripeSensitiveData {
        domain::StripeSensitiveData {
            api_secret_key: value.api_secret_key.clone(),
            webhook_secret: value.webhook_secret.clone(),
        }
    }

    pub fn connection_metadata_to_server(value: &ConnectionMeta) -> server::ConnectionMetadata {
        server::ConnectionMetadata {
            hubspot: conn_meta_items_to_server(&value.hubspot),
            pennylane: conn_meta_items_to_server(&value.pennylane),
        }
    }

    fn conn_meta_items_to_server(
        items: &Option<Vec<domain::ConnectionMetaItem>>,
    ) -> Vec<server::ConnectionMetadataItem> {
        items
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|item| server::ConnectionMetadataItem {
                connector_id: item.connector_id.as_proto(),
                external_id: item.external_id.clone(),
                sync_at: item.sync_at.naive_utc().as_proto(),
                external_company_id: item.external_company_id.clone(),
            })
            .collect()
    }
}
