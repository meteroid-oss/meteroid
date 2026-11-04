pub mod connectors {
    use crate::connectors::error::ConnectorApiError;
    use meteroid_app::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::connectors::v1 as server;
    use meteroid_grpc::meteroid::api::connectors::v1::HubspotConnectorData;
    use meteroid_store::adapters::payment::model::connector_webhook_url;
    use meteroid_store::adapters::payment::{
        HostedSetupCompletion, MandateSetupMode, PaymentCapabilities, payment_capabilities,
        requires_webhook_secret,
    };
    use meteroid_store::domain::connectors as domain;
    use meteroid_store::domain::connectors::{
        ConnectionMeta, ConnectorConfig, GocardlessEnvironment, PaymentCredentialsUpdate,
    };
    use meteroid_store::domain::enums as domain_enum;
    use meteroid_store::repositories::connectors::RoutedInvoicingEntity;
    use server::connect_payment_provider_request::Credentials;
    use server::update_payment_provider_credentials_request::Credentials as UpdateCredentials;

    /// Wire strings carrying credentials become secrets as soon as they enter the domain.
    fn secret(value: &str) -> secrecy::SecretString {
        secrecy::SecretString::from(value.to_string())
    }

    /// What an update actually carries: a blank wire string means "leave this one alone".
    fn given(value: &str) -> Option<String> {
        (!value.trim().is_empty()).then(|| value.trim().to_string())
    }

    fn given_secret(value: &str) -> Option<secrecy::SecretString> {
        given(value).map(secrecy::SecretString::from)
    }

    pub fn capabilities_to_server(
        caps: &PaymentCapabilities,
    ) -> server::PaymentProviderCapabilities {
        server::PaymentProviderCapabilities {
            supports_cards: caps.supports_cards(),
            supports_direct_debit: caps.supports_direct_debit(),
            mandate_setup_mode: match caps.mandate_setup_mode() {
                MandateSetupMode::EmbeddedClientSecret => {
                    server::MandateSetupMode::EmbeddedClientSecret
                }
                MandateSetupMode::HostedRedirect => server::MandateSetupMode::HostedRedirect,
                MandateSetupMode::EmbeddedDropIn => server::MandateSetupMode::EmbeddedDropIn,
            } as i32,
            // The wire enum has no "not a hosted flow" value, and WEBHOOK_BACKED is its zero:
            // that is what a provider without a hosted redirect reported before, and all the
            // frontend reads it for is whether a lost return needs polling.
            hosted_setup_completion: match caps.hosted_setup_completion() {
                Some(HostedSetupCompletion::PollingRequired) => {
                    server::HostedSetupCompletion::PollingRequired
                }
                Some(HostedSetupCompletion::WebhookBacked) | None => {
                    server::HostedSetupCompletion::WebhookBacked
                }
            } as i32,
            supports_hosted_invoice_payment: caps.supports_hosted_invoice_payment(),
            supports_hosted_checkout: caps.supports_hosted_checkout,
            asynchronous_settlement: caps.asynchronous_settlement,
            is_test_gateway: caps.is_test_gateway,
            supports_refunds: caps.supports_refunds(),
            supports_partial_refunds: caps.supports_partial_refunds(),
        }
    }

    fn payment_capabilities_to_server(
        provider: &domain_enum::ConnectorProviderEnum,
    ) -> Option<server::PaymentProviderCapabilities> {
        payment_capabilities(provider).map(capabilities_to_server)
    }

    /// The static capabilities, with the rail flags narrowed to what the account has enabled.
    fn effective_capabilities_to_server(
        connector: &domain::Connector,
    ) -> Option<server::PaymentProviderCapabilities> {
        payment_capabilities_to_server(&connector.provider()).map(|caps| {
            server::PaymentProviderCapabilities {
                supports_cards: connector.supports_card(),
                supports_direct_debit: connector.supports_direct_debit(),
                ..caps
            }
        })
    }

    pub fn connector_status_to_server(
        value: domain_enum::ConnectorStatusEnum,
    ) -> server::ConnectorStatusEnum {
        match value {
            domain_enum::ConnectorStatusEnum::Draft => server::ConnectorStatusEnum::Draft,
            domain_enum::ConnectorStatusEnum::Active => server::ConnectorStatusEnum::Active,
            domain_enum::ConnectorStatusEnum::Disabled => server::ConnectorStatusEnum::Disabled,
            domain_enum::ConnectorStatusEnum::Deleted => server::ConnectorStatusEnum::Deleted,
        }
    }

    fn payment_rail_to_server(
        value: &domain_enum::PaymentMethodTypeEnum,
    ) -> Option<server::PaymentRailEnum> {
        match value {
            domain_enum::PaymentMethodTypeEnum::Card => Some(server::PaymentRailEnum::Card),
            domain_enum::PaymentMethodTypeEnum::DirectDebitSepa => {
                Some(server::PaymentRailEnum::DirectDebitSepa)
            }
            domain_enum::PaymentMethodTypeEnum::DirectDebitAch => {
                Some(server::PaymentRailEnum::DirectDebitAch)
            }
            domain_enum::PaymentMethodTypeEnum::DirectDebitBacs => {
                Some(server::PaymentRailEnum::DirectDebitBacs)
            }
            domain_enum::PaymentMethodTypeEnum::Transfer
            | domain_enum::PaymentMethodTypeEnum::Other => None,
        }
    }

    fn provider_account_to_server(value: &domain::ProviderAccount) -> server::ProviderAccount {
        server::ProviderAccount {
            // The proto field is a plain string; a provider that exposes no account id
            // carried "" on the wire before, and still does.
            account_id: value.account_id.clone().unwrap_or_default(),
            display_name: value.display_name.clone(),
            livemode: value.livemode,
            key_hint: value.key_hint.clone(),
            methods: value
                .methods
                .iter()
                .filter_map(|m| {
                    Some(server::PaymentMethodStatus {
                        rail: payment_rail_to_server(&m.method)? as i32,
                        availability: match m.state {
                            domain::MethodState::Enabled => {
                                server::PaymentMethodAvailability::Enabled
                            }
                            domain::MethodState::Pending => {
                                server::PaymentMethodAvailability::Pending
                            }
                            domain::MethodState::Unavailable => {
                                server::PaymentMethodAvailability::Unavailable
                            }
                        } as i32,
                        reason: m.reason.clone(),
                    })
                })
                .collect(),
            checked_at: value.checked_at.as_proto(),
        }
    }

    pub fn connector_usage_to_server(
        value: &meteroid_store::repositories::connectors::ConnectorUsage,
    ) -> server::ConnectorUsage {
        let refs = |entities: &[RoutedInvoicingEntity]| {
            entities
                .iter()
                .map(|e| server::InvoicingEntityRef {
                    id: e.id.as_proto(),
                    name: e.legal_name.clone(),
                })
                .collect()
        };
        server::ConnectorUsage {
            routed_card: refs(&value.routed_card),
            routed_direct_debit: refs(&value.routed_direct_debit),
            customers_with_saved_methods: value.customers_with_saved_methods,
            active_subscriptions: value.active_subscriptions,
            in_flight_transactions: value.in_flight_transactions,
        }
    }

    /// Validates shape only; the store checks the credentials with the provider. Returns the
    /// alias alongside.
    pub fn credentials_to_domain(
        credentials: Credentials,
    ) -> Result<(String, PaymentCredentialsUpdate), ConnectorApiError> {
        Ok(match credentials {
            Credentials::Stripe(data) => {
                let sensitive = stripe_data_to_domain(&data);
                (
                    data.alias.clone(),
                    PaymentCredentialsUpdate::Stripe {
                        api_publishable_key: Some(data.api_publishable_key.clone()),
                        api_secret_key: Some(sensitive.api_secret_key),
                        webhook_secret: Some(sensitive.webhook_secret),
                    },
                )
            }
            Credentials::Gocardless(data) => {
                let (public, sensitive) = gocardless_data_to_domain(&data)?;
                (
                    data.alias,
                    PaymentCredentialsUpdate::Gocardless {
                        creditor_id: public.creditor_id,
                        environment: Some(public.environment),
                        access_token: Some(sensitive.access_token),
                        webhook_secret: Some(sensitive.webhook_secret),
                    },
                )
            }
            Credentials::Stancer(data) => (
                data.alias.clone(),
                PaymentCredentialsUpdate::Stancer {
                    api_secret_key: Some(stancer_data_to_domain(&data)?.api_secret_key),
                },
            ),
            Credentials::Mollie(data) => (
                data.alias.clone(),
                // The URL token is minted when credentials first land on the connector.
                PaymentCredentialsUpdate::Mollie {
                    api_key: Some(mollie_data_to_domain(&data)?.api_key),
                },
            ),
            // The store refuses the test gateway on a production tenant.
            Credentials::Mock(data) => (
                data.alias,
                PaymentCredentialsUpdate::Mock(domain::MockPublicData::default()),
            ),
        })
    }

    /// A rotation: the wire carries a blank string for "leave this one alone", which becomes
    /// `None` here. Non-blank values are still shape-checked.
    pub fn credentials_update_to_domain(
        credentials: UpdateCredentials,
    ) -> Result<PaymentCredentialsUpdate, ConnectorApiError> {
        Ok(match credentials {
            UpdateCredentials::Stripe(data) => PaymentCredentialsUpdate::Stripe {
                api_publishable_key: given(&data.api_publishable_key),
                api_secret_key: given_secret(&data.api_secret_key),
                webhook_secret: given_secret(&data.webhook_secret),
            },
            UpdateCredentials::Gocardless(data) => PaymentCredentialsUpdate::Gocardless {
                creditor_id: given(&data.creditor_id.clone().unwrap_or_default()),
                // A blank wire string still means "leave the stored environment alone"; anything
                // else has to be one of the two the domain enum knows.
                environment: match data.environment.as_str() {
                    "" => None,
                    other => Some(other.parse::<GocardlessEnvironment>().map_err(|_| {
                        ConnectorApiError::InvalidArgument(format!(
                            "GoCardless environment must be \"sandbox\" or \"live\", got {other:?}"
                        ))
                    })?),
                },
                access_token: given_secret(&data.access_token),
                webhook_secret: given_secret(&data.webhook_secret),
            },
            UpdateCredentials::Stancer(data) => PaymentCredentialsUpdate::Stancer {
                api_secret_key: given_secret(&data.api_secret_key),
            },
            UpdateCredentials::Mollie(data) => PaymentCredentialsUpdate::Mollie {
                // Shape-checked only when a new key is actually being set.
                api_key: match given(&data.api_key) {
                    None => None,
                    Some(_) => Some(mollie_data_to_domain(&data)?.api_key),
                },
            },
            // Nothing to rotate: the check re-reads the simulated account.
            UpdateCredentials::Mock(_) => {
                PaymentCredentialsUpdate::Mock(domain::MockPublicData::default())
            }
        })
    }

    /// The payment providers the API exposes; connecting anything else is refused.
    pub fn payment_provider_from_server(
        value: &server::ConnectorProviderEnum,
    ) -> Result<domain_enum::PaymentProvider, ConnectorApiError> {
        connector_provider_from_server(value)
            .as_payment()
            .ok_or_else(|| {
                ConnectorApiError::InvalidArgument(format!("{value:?} is not a payment provider"))
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
            server::ConnectorProviderEnum::Mock => domain_enum::ConnectorProviderEnum::Mock,
        }
    }

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
            domain_enum::ConnectorProviderEnum::Mock => Some(server::ConnectorProviderEnum::Mock),
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

    pub fn connector_meta_to_server(value: &domain::ConnectorMeta) -> Option<server::Connector> {
        let provider = connector_provider_to_server(&value.provider)?;
        Some(server::Connector {
            id: value.id.as_proto(),
            alias: value.alias.clone(),
            connector_type: connector_type_to_server(&value.connector_type) as i32,
            provider: provider as i32,
            data: None,
            payment_capabilities: payment_capabilities_to_server(&value.provider),
            status: connector_status_to_server(value.status) as i32,
            created_at: value.created_at.as_proto(),
            ..Default::default()
        })
    }

    pub fn connector_to_server(
        value: &domain::Connector,
        webhook_base_url: &str,
    ) -> Option<server::Connector> {
        let provider = connector_provider_to_server(&value.provider())?;
        let is_payment = value.connector_type() == domain_enum::ConnectorTypeEnum::PaymentProvider;
        let requires_secret = is_payment
            && value
                .provider()
                .as_payment()
                .is_some_and(requires_webhook_secret);
        Some(server::Connector {
            id: value.id.as_proto(),
            alias: value.alias.clone(),
            connector_type: connector_type_to_server(&value.connector_type()) as i32,
            provider: provider as i32,
            // Only the integrations expose public data; payment providers expose none.
            data: match &value.config {
                ConnectorConfig::Hubspot { data, .. } => {
                    data.as_ref().map(|d| server::ConnectorData {
                        data: Some(server::connector_data::Data::Hubspot(
                            HubspotConnectorData {
                                external_company_id: d.external_company_id.clone(),
                                auto_sync: d.auto_sync,
                            },
                        )),
                    })
                }
                ConnectorConfig::Pennylane { data, .. } => {
                    data.as_ref().map(|d| server::ConnectorData {
                        data: Some(server::connector_data::Data::Pennylane(
                            server::PennylaneConnectorData {
                                external_company_id: d.external_company_id.clone(),
                            },
                        )),
                    })
                }
                ConnectorConfig::Payment(_) => None,
            },
            payment_capabilities: effective_capabilities_to_server(value),
            status: connector_status_to_server(value.status) as i32,
            account: value.credentials.account().map(provider_account_to_server),
            // Only where the merchant has to configure it; Mollie's is built per payment.
            webhook_url: requires_secret
                .then(|| connector_webhook_url(webhook_base_url, value))
                .flatten(),
            created_at: value.created_at.as_proto(),
            has_credentials: value
                .payment_credentials()
                .is_some_and(|c| c.primary_secret().is_some()),
            has_webhook_secret: value.webhook_secret().is_some(),
            requires_webhook_secret: requires_secret,
            credentials_error: value.credentials.error().map(|e| server::CredentialsError {
                message: e.message.clone(),
                at: e.at.as_proto(),
            }),
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
        Ok(domain::MollieSensitiveData::new(secret(api_key)))
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
            api_secret_key: secret(&value.api_secret_key),
        })
    }

    pub fn stripe_data_to_domain(value: &server::StripeConnector) -> domain::StripeSensitiveData {
        domain::StripeSensitiveData {
            api_secret_key: secret(&value.api_secret_key),
            webhook_secret: secret(&value.webhook_secret),
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
        let environment = value
            .environment
            .parse::<GocardlessEnvironment>()
            .map_err(|_| {
                ConnectorApiError::InvalidArgument(format!(
                    "GoCardless environment must be \"sandbox\" or \"live\", got {:?}",
                    value.environment
                ))
            })?;
        Ok((
            domain::GocardlessPublicData {
                creditor_id: value.creditor_id.clone(),
                environment,
            },
            domain::GocardlessSensitiveData {
                access_token: secret(&value.access_token),
                webhook_secret: secret(&value.webhook_secret),
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
