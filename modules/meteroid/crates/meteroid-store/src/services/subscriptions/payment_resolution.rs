//! Resolves which payment methods are available for a subscription at runtime.
//! Connections are created on-demand at checkout time, not at subscription creation.

use crate::StoreResult;
use crate::domain::connectors::Connector;
use crate::domain::subscriptions::PaymentMethodsConfig;
use crate::domain::{Customer, InvoicingEntityProviderSensitive};
use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{BankAccountId, BaseId, CustomerConnectionId, TenantId};
use diesel_models::customer_connection::CustomerConnectionRow;
use error_stack::ResultExt;

#[derive(Debug, Clone, Default)]
pub struct ResolvedPaymentMethods {
    pub card_connection_id: Option<CustomerConnectionId>,
    pub direct_debit_connection_id: Option<CustomerConnectionId>,
    pub bank_account_id: Option<BankAccountId>,
    pub card_enabled: bool,
    pub direct_debit_enabled: bool,
    pub bank_transfer_enabled: bool,
}

impl ResolvedPaymentMethods {
    pub fn has_online_payment(&self) -> bool {
        self.card_connection_id.is_some() || self.direct_debit_connection_id.is_some()
    }

    pub fn has_any_payment_method(&self) -> bool {
        self.has_online_payment() || self.bank_account_id.is_some()
    }
}

impl Services {
    pub async fn resolve_payment_methods(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        payment_methods_config: Option<&PaymentMethodsConfig>,
        customer: &Customer,
        invoicing_entity_providers: &InvoicingEntityProviderSensitive,
    ) -> StoreResult<ResolvedPaymentMethods> {
        // None defaults to Online with all providers
        let config = payment_methods_config
            .cloned()
            .unwrap_or_else(PaymentMethodsConfig::online);

        match config {
            PaymentMethodsConfig::Online { config } => {
                self.resolve_online_payment_methods(
                    conn,
                    tenant_id,
                    customer,
                    invoicing_entity_providers,
                    config.as_ref(),
                )
                .await
            }
            PaymentMethodsConfig::BankTransfer { account_id } => {
                self.resolve_bank_transfer_payment_methods(invoicing_entity_providers, account_id)
            }
            PaymentMethodsConfig::External => Ok(ResolvedPaymentMethods::default()),
        }
    }

    async fn resolve_online_payment_methods(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer: &Customer,
        invoicing_entity_providers: &InvoicingEntityProviderSensitive,
        online_config: Option<&crate::domain::subscriptions::OnlineMethodsConfig>,
    ) -> StoreResult<ResolvedPaymentMethods> {
        let card_enabled = online_config
            .and_then(|c| c.card.as_ref())
            .map(|m| m.enabled)
            .unwrap_or(true); // Default: enabled if no config

        let direct_debit_enabled = online_config
            .and_then(|c| c.direct_debit.as_ref())
            .map(|m| m.enabled)
            .unwrap_or(true); // Default: enabled if no config

        if !card_enabled && !direct_debit_enabled {
            return Ok(ResolvedPaymentMethods {
                card_enabled: false,
                direct_debit_enabled: false,
                bank_transfer_enabled: false,
                ..Default::default()
            });
        }

        let existing_connections =
            CustomerConnectionRow::list_connections_by_customer_id(conn, &tenant_id, &customer.id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let mut card_connection_id = None;
        let mut direct_debit_connection_id = None;

        if card_enabled && let Some(provider) = invoicing_entity_providers.card_provider.as_ref() {
            card_connection_id = self
                .get_or_create_connection_for_provider(
                    conn,
                    customer,
                    provider,
                    &existing_connections,
                )
                .await?;
        }

        if direct_debit_enabled
            && let Some(provider) = invoicing_entity_providers.direct_debit_provider.as_ref()
        {
            if card_connection_id.is_some()
                && invoicing_entity_providers
                    .card_provider
                    .as_ref()
                    .is_some_and(|cp| cp.id == provider.id)
            {
                direct_debit_connection_id = card_connection_id;
            } else {
                direct_debit_connection_id = self
                    .get_or_create_connection_for_provider(
                        conn,
                        customer,
                        provider,
                        &existing_connections,
                    )
                    .await?;
            }
        }

        Ok(ResolvedPaymentMethods {
            card_connection_id,
            direct_debit_connection_id,
            bank_account_id: None,
            card_enabled,
            direct_debit_enabled,
            bank_transfer_enabled: false,
        })
    }

    fn resolve_bank_transfer_payment_methods(
        &self,
        invoicing_entity_providers: &InvoicingEntityProviderSensitive,
        account_id_override: Option<BankAccountId>,
    ) -> StoreResult<ResolvedPaymentMethods> {
        let bank_account_id = account_id_override.or_else(|| {
            invoicing_entity_providers
                .bank_account
                .as_ref()
                .map(|ba| ba.id)
        });

        Ok(ResolvedPaymentMethods {
            card_connection_id: None,
            direct_debit_connection_id: None,
            bank_account_id,
            card_enabled: false,
            direct_debit_enabled: false,
            bank_transfer_enabled: bank_account_id.is_some(),
        })
    }

    async fn get_or_create_connection_for_provider(
        &self,
        conn: &mut PgConn,
        customer: &Customer,
        provider: &Connector,
        existing_connections: &[CustomerConnectionRow],
    ) -> StoreResult<Option<CustomerConnectionId>> {
        use crate::adapters::payment_service_providers::initialize_payment_provider;

        if let Some(existing) = existing_connections
            .iter()
            .find(|c| c.connector_id == provider.id)
        {
            return Ok(Some(existing.id));
        }

        let payment_provider = initialize_payment_provider(provider)
            .change_context(StoreError::PaymentProviderError)?;

        let external_id = payment_provider
            .create_customer_in_provider(customer, provider)
            .await
            .change_context(StoreError::PaymentProviderError)?;

        let new_connection = CustomerConnectionRow {
            id: CustomerConnectionId::new(),
            customer_id: customer.id,
            connector_id: provider.id,
            external_customer_id: external_id,
            supported_payment_types: None,
        };

        let inserted = new_connection
            .insert(conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(Some(inserted.id))
    }
}
