use crate::StoreResult;
use crate::adapters::payment_service_providers::initialize_payment_provider;
use crate::domain::connectors::Connector;
use crate::domain::{CustomerConnection, PaymentMethodTypeEnum, SetupIntent};
use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{BaseId, CustomerConnectionId, TenantId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::invoicing_entities::{InvoicingEntityProvidersRow, InvoicingEntityRow};
use error_stack::ResultExt;
use common_domain::country::CountryCode;
use crate::services::payment::sepa::SEPA_COUNTRIES;

/// Helper function to determine which direct debit payment methods are supported
/// based on the invoicing entity's country
fn get_direct_debit_types_for_country(country: &CountryCode) -> Vec<Option<diesel_models::enums::PaymentMethodTypeEnum>> {
    let Some(iso_country_code) = rust_iso3166::from_alpha2(&country.code) else {
        log::warn!("Invalid country code: {}", country.code);
        return vec![];
    };

    if iso_country_code == rust_iso3166::US || iso_country_code == rust_iso3166::CA {
        vec![Some(diesel_models::enums::PaymentMethodTypeEnum::DirectDebitAch)]
    } else if SEPA_COUNTRIES.contains(&iso_country_code) {
        vec![Some(diesel_models::enums::PaymentMethodTypeEnum::DirectDebitSepa)]
    } else {
        vec![]
    }
}

impl Services {
    /// Gets existing or creates new customer connections for card and direct debit providers
    /// This ensures customers can add payment methods even if they don't have connections yet
    pub(in crate::services) async fn get_or_create_customer_connections(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: common_domain::ids::CustomerId,
        invoicing_entity_id: common_domain::ids::InvoicingEntityId,
    ) -> StoreResult<(Option<CustomerConnectionId>, Option<CustomerConnectionId>)> {
        use crate::repositories::CustomersInterface;
        use diesel_models::customer_connection::CustomerConnectionRow;

        // Get customer details
        let customer = self.store.find_customer_by_id(customer_id, tenant_id).await?;

        // Get invoicing entity to determine country for direct debit payment types
        let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
            conn,
            invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        // Get invoicing entity providers
        let providers = diesel_models::invoicing_entities::InvoicingEntityProvidersRow::resolve_providers_by_id(
            conn,
            invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        let providers_sensitive = crate::domain::InvoicingEntityProviderSensitive::from_row(
            providers,
            &self.store.settings.crypt_key,
        )?;

        // Get existing customer connections
        let existing_connections = diesel_models::customer_connection::CustomerConnectionRow::list_connections_by_customer_id(
            conn,
            &tenant_id,
            &customer_id,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        let mut card_connection_id = None;
        let mut direct_debit_connection_id = None;

        // Check if the same provider is used for both card and direct debit
        let same_provider = match (&providers_sensitive.card_provider, &providers_sensitive.direct_debit_provider) {
            (Some(card), Some(dd)) => card.id == dd.id,
            _ => false,
        };

        if same_provider {
            // Same provider handles both card and direct debit
            // Create a single connection with combined payment types
            let provider = providers_sensitive.card_provider.as_ref().unwrap();

            // Check if connection already exists
            let existing = existing_connections
                .iter()
                .find(|c| c.connector_id == provider.id);

            let connection_id = if let Some(conn_row) = existing {
                conn_row.id
            } else {
                // Create new customer in payment provider
                let payment_provider = initialize_payment_provider(provider)
                    .change_context(StoreError::PaymentProviderError)?;

                let external_id = payment_provider
                    .create_customer_in_provider(&customer, provider)
                    .await
                    .change_context(StoreError::PaymentProviderError)?;

                // Combine payment types: Card + appropriate direct debit types based on country
                let mut payment_types = vec![Some(diesel_models::enums::PaymentMethodTypeEnum::Card)];
                payment_types.extend(get_direct_debit_types_for_country(&invoicing_entity.country));

                // Create connection in our database
                let new_connection = CustomerConnectionRow {
                    id: CustomerConnectionId::new(),
                    customer_id,
                    connector_id: provider.id,
                    external_customer_id: external_id,
                    supported_payment_types: Some(payment_types),
                };

                let inserted = new_connection
                    .insert(conn)
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?;

                inserted.id
            };

            // Use the same connection ID for both
            card_connection_id = Some(connection_id);
            direct_debit_connection_id = Some(connection_id);
        } else {
            // Different providers for card and direct debit - create separate connections

            // Check for card provider connection
            if let Some(card_provider) = &providers_sensitive.card_provider {
                // Check if connection already exists
                let existing = existing_connections
                    .iter()
                    .find(|c| c.connector_id == card_provider.id);

                if let Some(conn_row) = existing {
                    card_connection_id = Some(conn_row.id);
                } else {
                    // Create new customer in payment provider
                    let provider = initialize_payment_provider(card_provider)
                        .change_context(StoreError::PaymentProviderError)?;

                    let external_id = provider
                        .create_customer_in_provider(&customer, card_provider)
                        .await
                        .change_context(StoreError::PaymentProviderError)?;

                    // Create connection in our database with Card payment type
                    let new_connection = CustomerConnectionRow {
                        id: CustomerConnectionId::new(),
                        customer_id,
                        connector_id: card_provider.id,
                        external_customer_id: external_id,
                        supported_payment_types: Some(vec![Some(diesel_models::enums::PaymentMethodTypeEnum::Card)]),
                    };

                    let inserted = new_connection
                        .insert(conn)
                        .await
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                    card_connection_id = Some(inserted.id);
                }
            }

            // Check for direct debit provider connection
            if let Some(direct_debit_provider) = &providers_sensitive.direct_debit_provider {
                // Check if connection already exists
                let existing = existing_connections
                    .iter()
                    .find(|c| c.connector_id == direct_debit_provider.id);

                if let Some(conn_row) = existing {
                    direct_debit_connection_id = Some(conn_row.id);
                } else {
                    // Create new customer in payment provider
                    let provider = initialize_payment_provider(direct_debit_provider)
                        .change_context(StoreError::PaymentProviderError)?;

                    let external_id = provider
                        .create_customer_in_provider(&customer, direct_debit_provider)
                        .await
                        .change_context(StoreError::PaymentProviderError)?;

                    // Create connection in our database with country-specific direct debit types
                    let new_connection = CustomerConnectionRow {
                        id: CustomerConnectionId::new(),
                        customer_id,
                        connector_id: direct_debit_provider.id,
                        external_customer_id: external_id,
                        supported_payment_types: Some(get_direct_debit_types_for_country(&invoicing_entity.country)),
                    };

                    let inserted = new_connection
                        .insert(conn)
                        .await
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                    direct_debit_connection_id = Some(inserted.id);
                }
            }
        }

        Ok((card_connection_id, direct_debit_connection_id))
    }

    pub(in crate::services) async fn create_setup_intent_for_type(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
        connection_type: crate::domain::ConnectionTypeEnum,
    ) -> StoreResult<SetupIntent> {
        self.create_setup_intent_internal(conn, tenant_id, customer_connection_id, Some(connection_type)).await
    }

    pub(in crate::services) async fn create_setup_intent(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<SetupIntent> {
        self.create_setup_intent_internal(conn, tenant_id, customer_connection_id, None).await
    }

    async fn create_setup_intent_internal(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
        requested_connection_type: Option<crate::domain::ConnectionTypeEnum>,
    ) -> StoreResult<SetupIntent> {
        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, tenant_id, customer_connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let customer_connection: CustomerConnection = CustomerConnection {
            id: connection.id,
            customer_id: connection.customer.id,
            connector_id: connection.connector.id,
            supported_payment_types: connection
                .supported_payment_types
                .as_ref()
                .map(|v| v.iter().flatten().map(|t| t.clone().into()).collect()),
            external_customer_id: connection.external_customer_id,
        };


        log::info!("DEBUG -- customer_connection: {:#?}", customer_connection);


        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;

        let provider = initialize_payment_provider(&connector)
            .change_context(StoreError::PaymentProviderError)?;


        // payment methods for that connector are either retrieved from invoicing entity (default) or overridden through the connection
        let mut payment_methods = match connection.supported_payment_types {
            Some(types) if !types.is_empty() => types
                .into_iter()
                .filter_map(|t| t.map(Into::<PaymentMethodTypeEnum>::into))
                .collect(),
            _ => {
                let invoicing_entity_providers =
                    InvoicingEntityProvidersRow::resolve_providers_by_id(
                        conn,
                        connection.customer.invoicing_entity_id,
                        *tenant_id,
                    )
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?;

                let mut payment_methods = Vec::new();
                if let Some(card_provider) = invoicing_entity_providers.card_provider
                    && card_provider.id == connector.id
                {
                    payment_methods.push(PaymentMethodTypeEnum::Card);
                }
                if let Some(direct_debit_provider) = invoicing_entity_providers.direct_debit_provider
                    && direct_debit_provider.id == connector.id {
                        let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                            conn,
                            connection.customer.invoicing_entity_id,
                            *tenant_id,
                        )
                        .await
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                        // Use the helper function to determine direct debit types based on country
                        let direct_debit_types = get_direct_debit_types_for_country(&invoicing_entity.country);
                        payment_methods.extend(
                            direct_debit_types
                                .into_iter()
                                .filter_map(|t| t.map(Into::<PaymentMethodTypeEnum>::into))
                        );

                }

                log::info!("DEBUG -- payment_methods: {:#?}", payment_methods);

                payment_methods
            }

        };

        // Filter payment methods based on requested connection type if specified
        if let Some(requested_type) = requested_connection_type {
            payment_methods.retain(|pm| match requested_type {
                crate::domain::ConnectionTypeEnum::Card => matches!(pm, PaymentMethodTypeEnum::Card),
                crate::domain::ConnectionTypeEnum::DirectDebit => matches!(
                    pm,
                    PaymentMethodTypeEnum::DirectDebitSepa
                        | PaymentMethodTypeEnum::DirectDebitAch
                        | PaymentMethodTypeEnum::DirectDebitBacs
                ),
            });
        }

        let setup_intent = provider
            .create_setup_intent_in_provider(&customer_connection, &connector, payment_methods)
            .await
            .change_context_lazy(|| StoreError::PaymentProviderError)?;

        Ok(setup_intent)
    }
}
