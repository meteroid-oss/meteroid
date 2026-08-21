//! Resolves which payment methods are available for a subscription at runtime.
//! Connections are created on-demand at checkout time, not at subscription creation.

use crate::StoreResult;
use crate::adapters::payment::error::ConnectorError;
use crate::domain::connectors::Connector;
use crate::domain::enums::PaymentMethodTypeEnum;
use crate::domain::subscriptions::PaymentMethodsConfig;
use crate::domain::{Customer, CustomerPaymentMethod, InvoicingEntityProviderSensitive};
use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{BankAccountId, BaseId, CustomerConnectionId, TenantId};
use diesel_models::customer_connection::CustomerConnectionRow;
use error_stack::{Report, ResultExt};

use std::time::Duration;
const PAYMENT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Default)]
pub struct ResolvedPaymentMethods {
    pub card_connection_id: Option<CustomerConnectionId>,
    pub direct_debit_connection_id: Option<CustomerConnectionId>,
    pub bank_account_id: Option<BankAccountId>,
    pub card_enabled: bool,
    pub direct_debit_enabled: bool,
    pub bank_transfer_enabled: bool,
    /// Set when a method is configured but its provider connection could not be
    /// established (e.g. GoCardless rejected `create_customer` for a customer
    /// with no email). Surfaced to the payment page so the customer sees *why*
    /// the method is missing instead of it silently vanishing. Recomputed each
    /// load, so it clears itself once the underlying data is fixed.
    pub card_unavailable_reason: Option<String>,
    pub direct_debit_unavailable_reason: Option<String>,
}

impl ResolvedPaymentMethods {
    pub fn has_online_payment(&self) -> bool {
        self.card_connection_id.is_some() || self.direct_debit_connection_id.is_some()
    }

    pub fn has_any_payment_method(&self) -> bool {
        self.has_online_payment() || self.bank_account_id.is_some()
    }

    /// Filters a list of customer payment methods to only include those that are
    /// usable based on the resolved connection IDs.
    ///
    /// - Card methods are included only if they belong to the resolved card connection
    /// - Direct debit methods are included only if they belong to the resolved DD connection
    /// - Other payment method types (Transfer, Other) are excluded as they're not usable for online payment
    pub fn filter_payment_methods(
        &self,
        payment_methods: Vec<CustomerPaymentMethod>,
    ) -> Vec<CustomerPaymentMethod> {
        payment_methods
            .into_iter()
            .filter(|pm| self.is_payment_method_usable(pm))
            .collect()
    }

    /// Checks if a specific payment method is usable based on the resolved connections.
    pub fn is_payment_method_usable(&self, pm: &CustomerPaymentMethod) -> bool {
        match pm.payment_method_type {
            PaymentMethodTypeEnum::Card => self.card_connection_id == Some(pm.connection_id),
            PaymentMethodTypeEnum::DirectDebitSepa
            | PaymentMethodTypeEnum::DirectDebitAch
            | PaymentMethodTypeEnum::DirectDebitBacs => {
                self.direct_debit_connection_id == Some(pm.connection_id)
            }
            // Transfer and Other are not usable for online payment
            PaymentMethodTypeEnum::Transfer | PaymentMethodTypeEnum::Other => false,
        }
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
        let mut card_unavailable_reason = None;
        let mut direct_debit_unavailable_reason = None;

        if card_enabled && let Some(provider) = invoicing_entity_providers.card_provider.as_ref() {
            // A provider failure here (e.g. GoCardless rejecting `create_customer`)
            // must NEVER crash the checkout / invoice-payment page. Degrade the
            // affected method to "unavailable" and let the other providers render.
            (card_connection_id, card_unavailable_reason) = self
                .resolve_connection_or_degrade(
                    conn,
                    customer,
                    provider,
                    &existing_connections,
                    "card",
                )
                .await;
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
                (direct_debit_connection_id, direct_debit_unavailable_reason) = self
                    .resolve_connection_or_degrade(
                        conn,
                        customer,
                        provider,
                        &existing_connections,
                        "direct_debit",
                    )
                    .await;
            }
        }

        Ok(ResolvedPaymentMethods {
            card_connection_id,
            direct_debit_connection_id,
            bank_account_id: None,
            card_enabled,
            direct_debit_enabled,
            bank_transfer_enabled: false,
            card_unavailable_reason,
            direct_debit_unavailable_reason,
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
            card_unavailable_reason: None,
            direct_debit_unavailable_reason: None,
        })
    }

    /// Wraps [`Self::get_or_create_connection_for_provider`] so a single
    /// provider's failure degrades that payment method to "unavailable" instead
    /// of failing the whole payment-page load. The error is logged with enough
    /// context (customer, provider, method slot) to debug, then swallowed —
    /// the customer still sees the invoice and any working payment methods.
    async fn resolve_connection_or_degrade(
        &self,
        conn: &mut PgConn,
        customer: &Customer,
        provider: &Connector,
        existing_connections: &[CustomerConnectionRow],
        method_slot: &str,
    ) -> (Option<CustomerConnectionId>, Option<String>) {
        match self
            .get_or_create_connection_for_provider(conn, customer, provider, existing_connections)
            .await
        {
            Ok(connection_id) => (connection_id, None),
            Err(err) => {
                // Full technical detail (provider, request_id, stack) goes to the
                // logs; a trimmed, customer-safe reason goes to the UI.
                log::error!(
                    "Payment method '{method_slot}' unavailable for customer {} via {:?} connector {}: {err:?}",
                    customer.id.as_base62(),
                    provider.provider,
                    provider.id.as_base62(),
                );
                (None, Some(customer_facing_reason(&err)))
            }
        }
    }

    async fn get_or_create_connection_for_provider(
        &self,
        conn: &mut PgConn,
        customer: &Customer,
        provider: &Connector,
        existing_connections: &[CustomerConnectionRow],
    ) -> StoreResult<Option<CustomerConnectionId>> {
        use crate::adapters::payment::initialize_payment_connector;
        use crate::adapters::payment::model::{CreateCustomerRequest, IdempotencyKey};

        if let Some(existing) = existing_connections
            .iter()
            .find(|c| c.connector_id == provider.id)
        {
            return Ok(Some(existing.id));
        }

        let connector_impl = initialize_payment_connector(provider)
            .change_context(StoreError::PaymentProviderError)?;

        // Idempotency: customer × provider. Retry after a network blip
        // returns the original external customer rather than creating a duplicate.
        let request = CreateCustomerRequest {
            idempotency_key: IdempotencyKey::new(format!(
                "customer:{}:{}",
                customer.id.as_base62(),
                provider.id.as_base62()
            )),
        };

        let external_ref = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.create_customer(provider, customer, request),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?
        .change_context(StoreError::PaymentProviderError)?;

        let external_id = external_ref.external_id;

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

/// Extract a concise, customer-safe reason from a connection-setup failure.
/// The provider's own error usually carries the actionable detail (e.g. an
/// "email is required" validation message); we surface that while stripping
/// internal noise (the `ConnectorError` variant prefix and the `request_id`).
fn customer_facing_reason(err: &Report<StoreError>) -> String {
    match err.downcast_ref::<ConnectorError>() {
        Some(connector_err) => clean_reason(&connector_err.to_string()),
        // Non-provider failures (timeout, DB) shouldn't leak internals.
        None => {
            "This payment method is temporarily unavailable. Please try again later.".to_string()
        }
    }
}

fn clean_reason(msg: &str) -> String {
    // Drop the internal request_id parenthetical if present.
    let msg = msg
        .split_once(" (request_id=")
        .map(|(head, _)| head)
        .unwrap_or(msg);

    // Drop the internal `ConnectorError` variant prefix.
    for prefix in [
        "Customer operation failed: ",
        "Mandate setup failed: ",
        "Charge failed: ",
    ] {
        if let Some(rest) = msg.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }

    msg.trim().to_string()
}

#[cfg(test)]
mod reason_tests {
    use super::clean_reason;

    #[test]
    fn strips_variant_prefix_and_request_id() {
        let raw = "Customer operation failed: gocardless rejected: Validation failed [email: is required] (request_id=REQ0123)";
        assert_eq!(
            clean_reason(raw),
            "gocardless rejected: Validation failed [email: is required]"
        );
    }

    #[test]
    fn leaves_plain_message_untouched() {
        assert_eq!(clean_reason("something broke"), "something broke");
    }
}
