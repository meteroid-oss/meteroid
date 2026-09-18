pub mod connectors {
    use crate::api::connectors::error::ConnectorApiError;
    use crate::api::shared::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::connectors::v1 as server;
    use meteroid_grpc::meteroid::api::connectors::v1::HubspotConnectorData;
    use meteroid_store::adapters::payment::{
        ConnectorCapabilities, HostedSetupCompletion, MandateSetupMode, provider_capabilities,
    };
    use meteroid_store::domain::connectors as domain;
    use meteroid_store::domain::connectors::{ConnectionMeta, ProviderData, ProviderSensitiveData};
    use meteroid_store::domain::enums as domain_enum;
    use server::connect_payment_provider_request::Credentials;

    pub fn capabilities_to_server(
        caps: &ConnectorCapabilities,
    ) -> server::PaymentProviderCapabilities {
        server::PaymentProviderCapabilities {
            supports_cards: caps.supports_cards,
            supports_direct_debit: caps.supports_direct_debit(),
            mandate_setup_mode: match caps.mandate_setup_mode {
                MandateSetupMode::EmbeddedClientSecret => {
                    server::MandateSetupMode::EmbeddedClientSecret
                }
                MandateSetupMode::HostedRedirect => server::MandateSetupMode::HostedRedirect,
                MandateSetupMode::EmbeddedDropIn => server::MandateSetupMode::EmbeddedDropIn,
            } as i32,
            hosted_setup_completion: match caps.hosted_setup_completion {
                HostedSetupCompletion::WebhookBacked => {
                    server::HostedSetupCompletion::WebhookBacked
                }
                HostedSetupCompletion::PollingRequired => {
                    server::HostedSetupCompletion::PollingRequired
                }
            } as i32,
            supports_hosted_invoice_payment: caps.supports_hosted_invoice_payment,
            supports_hosted_checkout: caps.supports_hosted_checkout,
            asynchronous_settlement: caps.asynchronous_settlement,
        }
    }

    fn payment_capabilities_to_server(
        provider: &domain_enum::ConnectorProviderEnum,
    ) -> Option<server::PaymentProviderCapabilities> {
        provider_capabilities(provider).map(capabilities_to_server)
    }

    /// A payment provider's credentials as the store persists them.
    pub struct PaymentProviderCredentials {
        pub provider: domain_enum::ConnectorProviderEnum,
        pub alias: String,
        pub data: ProviderData,
        pub sensitive: ProviderSensitiveData,
    }

    /// Validates shape only; `CredentialOps::validate_credentials` checks them with the provider.
    pub fn credentials_to_domain(
        credentials: Credentials,
    ) -> Result<PaymentProviderCredentials, ConnectorApiError> {
        Ok(match credentials {
            Credentials::Stripe(data) => PaymentProviderCredentials {
                provider: domain_enum::ConnectorProviderEnum::Stripe,
                alias: data.alias.clone(),
                // The account id is filled in by the credential check.
                data: ProviderData::Stripe(domain::StripePublicData {
                    api_publishable_key: data.api_publishable_key.clone(),
                    account_id: String::new(),
                }),
                sensitive: ProviderSensitiveData::Stripe(stripe_data_to_domain(&data)),
            },
            Credentials::Gocardless(data) => {
                let (public, sensitive) = gocardless_data_to_domain(&data)?;
                PaymentProviderCredentials {
                    provider: domain_enum::ConnectorProviderEnum::Gocardless,
                    alias: data.alias,
                    data: ProviderData::Gocardless(public),
                    sensitive: ProviderSensitiveData::Gocardless(sensitive),
                }
            }
            Credentials::Stancer(data) => PaymentProviderCredentials {
                provider: domain_enum::ConnectorProviderEnum::Stancer,
                alias: data.alias.clone(),
                data: ProviderData::Stancer(domain::StancerPublicData::default()),
                sensitive: ProviderSensitiveData::Stancer(stancer_data_to_domain(&data)?),
            },
            Credentials::Mollie(data) => PaymentProviderCredentials {
                provider: domain_enum::ConnectorProviderEnum::Mollie,
                alias: data.alias.clone(),
                data: ProviderData::Mollie(domain::MolliePublicData::default()),
                sensitive: ProviderSensitiveData::Mollie(mollie_data_to_domain(&data)?),
            },
        })
    }

    pub fn connector_provider_from_server(
        value: &server::ConnectorProviderEnum,
    ) -> domain_enum::ConnectorProviderEnum {
        match *value {
            server::ConnectorProviderEnum::Stripe => domain_enum::ConnectorProviderEnum::Stripe,
            server::ConnectorProviderEnum::Hubspot => domain_enum::ConnectorProviderEnum::Hubspot,
            server::ConnectorProviderEnum::Pennylane => {
                domain_enum::ConnectorProviderEnum::Pennylane
            }
            server::ConnectorProviderEnum::Gocardless => {
                domain_enum::ConnectorProviderEnum::Gocardless
            }
            server::ConnectorProviderEnum::Stancer => domain_enum::ConnectorProviderEnum::Stancer,
            server::ConnectorProviderEnum::Mollie => domain_enum::ConnectorProviderEnum::Mollie,
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
            domain_enum::ConnectorProviderEnum::Gocardless => {
                Some(server::ConnectorProviderEnum::Gocardless)
            }
            domain_enum::ConnectorProviderEnum::Stancer => {
                Some(server::ConnectorProviderEnum::Stancer)
            }
            domain_enum::ConnectorProviderEnum::Mollie => {
                Some(server::ConnectorProviderEnum::Mollie)
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
            server::ConnectorTypeEnum::Tax => domain_enum::ConnectorTypeEnum::Tax,
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
            domain_enum::ConnectorTypeEnum::Tax => server::ConnectorTypeEnum::Tax,
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
            payment_capabilities: payment_capabilities_to_server(&value.provider),
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
                // GoCardless public data not yet exposed via proto. The
                // connector still works end-to-end; the proto enumeration
                // layer just hasn't been regenerated to know about it.
                ProviderData::Gocardless(_) => None,
                // Stancer has no public data to expose (no publishable key,
                // no external account id).
                ProviderData::Stancer(_) => None,
                // Mollie has no public data to expose.
                ProviderData::Mollie(_) => None,
            }),
            payment_capabilities: payment_capabilities_to_server(&value.provider),
        })
    }

    /// The API key is the only credential; the webhook token is generated by the domain type.
    pub fn mollie_data_to_domain(
        value: &server::MollieConnector,
    ) -> Result<domain::MollieSensitiveData, ConnectorApiError> {
        let api_key = value.api_key.trim();
        if api_key.is_empty() {
            return Err(ConnectorApiError::InvalidInput(
                "Mollie api_key must not be empty".to_string(),
            ));
        }
        if api_key.starts_with("access_") {
            return Err(ConnectorApiError::InvalidInput(
                "Mollie access tokens are not supported yet; use a standard API key (test_… or live_…)"
                    .to_string(),
            ));
        }
        if !(api_key.starts_with("test_") || api_key.starts_with("live_")) {
            return Err(ConnectorApiError::InvalidInput(
                "Mollie api_key must start with test_ or live_".to_string(),
            ));
        }
        Ok(domain::MollieSensitiveData::new(api_key.to_string()))
    }

    /// Stancer has no public data — the secret key is the whole configuration
    /// (mode rides on the key prefix). Rejects an empty key rather than
    /// persisting an unusable connector.
    pub fn stancer_data_to_domain(
        value: &server::StancerConnector,
    ) -> Result<domain::StancerSensitiveData, ConnectorApiError> {
        if value.api_secret_key.trim().is_empty() {
            return Err(ConnectorApiError::InvalidArgument(
                "Stancer api_secret_key must not be empty".to_string(),
            ));
        }
        Ok(domain::StancerSensitiveData {
            api_secret_key: value.api_secret_key.clone(),
        })
    }

    pub fn stripe_data_to_domain(value: &server::StripeConnector) -> domain::StripeSensitiveData {
        domain::StripeSensitiveData {
            api_secret_key: value.api_secret_key.clone(),
            webhook_secret: value.webhook_secret.clone(),
            // Set when we auto-register an endpoint via WebhookOps; left None
            // when the customer pasted the secret manually.
            webhook_endpoint_id: None,
        }
    }

    /// Split the proto `GoCardlessConnector` payload into the (public,
    /// sensitive) domain pair the connectors repository expects. The proto
    /// flattens everything into one message; we partition by what should be
    /// encrypted at rest (access_token + webhook_secret) vs. what shows up
    /// in the dashboard (creditor_id, environment).
    ///
    /// Validates the credentials are present and `environment` is exactly one
    /// of the two values the frontend sends; anything else is rejected rather
    /// than silently coerced.
    pub fn gocardless_data_to_domain(
        value: &server::GoCardlessConnector,
    ) -> Result<
        (
            domain::GocardlessPublicData,
            domain::GocardlessSensitiveData,
        ),
        ConnectorApiError,
    > {
        if value.access_token.trim().is_empty() {
            return Err(ConnectorApiError::InvalidArgument(
                "GoCardless access_token must not be empty".to_string(),
            ));
        }
        if value.webhook_secret.trim().is_empty() {
            return Err(ConnectorApiError::InvalidArgument(
                "GoCardless webhook_secret must not be empty".to_string(),
            ));
        }
        let environment = match value.environment.as_str() {
            "sandbox" | "live" => value.environment.clone(),
            other => {
                return Err(ConnectorApiError::InvalidArgument(format!(
                    "GoCardless environment must be \"sandbox\" or \"live\", got {other:?}"
                )));
            }
        };
        Ok((
            domain::GocardlessPublicData {
                creditor_id: value.creditor_id.clone(),
                environment,
            },
            domain::GocardlessSensitiveData {
                access_token: value.access_token.clone(),
                webhook_secret: value.webhook_secret.clone(),
            },
        ))
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

    #[cfg(test)]
    mod tests {
        use super::*;

        fn mollie(api_key: &str) -> Result<domain::MollieSensitiveData, ConnectorApiError> {
            mollie_data_to_domain(&server::MollieConnector {
                alias: "mollie".into(),
                api_key: api_key.into(),
            })
        }

        #[test]
        fn mollie_accepts_standard_api_keys_only() {
            assert!(mollie(" test_abc ").is_ok());
            assert!(mollie("live_abc").is_ok());
            let Err(ConnectorApiError::InvalidInput(msg)) = mollie("access_abc") else {
                panic!("access token must be rejected");
            };
            assert!(msg.contains("access tokens are not supported"));
            assert!(mollie("sk_abc").is_err());
        }
    }
}
