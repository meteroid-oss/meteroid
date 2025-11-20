use crate::domain::enums::{ConnectorTypeEnum, PaymentMethodTypeEnum, PlanTypeEnum};
use crate::domain::{
    Customer, CustomerConnection, InvoicingEntityProviderSensitive, SubscriptionNew,
    SubscriptionPaymentStrategy,
};
use crate::errors::StoreError;

use super::context::SubscriptionCreationContext;
use crate::adapters::payment_service_providers::initialize_payment_provider;
use crate::domain::connectors::Connector;
use crate::store::PgConn;
use crate::{StoreResult, services::Services};
use common_domain::ids::{
    BankAccountId, BaseId, ConnectorId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId,
};
use diesel_models::customer_connection::CustomerConnectionRow;
use error_stack::{Report, ResultExt};

impl Services {
    /// Sets up the appropriate payment provider for a subscription
    ///
    /// This function determines the payment setup strategy based on:
    /// - The subscription's payment strategy (Auto, Bank, External)
    /// - The customer's existing payment methods
    /// - The invoicing entity's available payment providers
    ///
    /// The result may include:
    /// - An existing payment method to use
    /// - A checkout session to collect payment details
    /// - A bank account for direct transfers
    /// - An external payment flag (manual payments)
    pub(crate) async fn setup_payment_provider(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionNew,
        customer: &Customer,
        context: &SubscriptionCreationContext,
    ) -> StoreResult<PaymentSetupResult> {
        // Find the plan for this subscription
        let plan = context
            .plans
            .iter()
            .find(|p| p.version_id == subscription.plan_version_id)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(
                    "No plan found for subscription".to_string(),
                ))
            })?;

        // Free plans don't need payment setup
        if plan.plan_type == PlanTypeEnum::Free {
            return Ok(PaymentSetupResult::external());
        }

        // Determine payment strategy, defaulting to Auto if not specified
        let strategy = subscription
            .payment_strategy
            .clone()
            .unwrap_or(SubscriptionPaymentStrategy::Auto);

        // Get invoicing entity payment providers for the customer
        let invoicing_entity_providers = context
            .get_invoicing_entity_providers_for_customer(customer)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(
                    "No invoicing entity found for customer".to_string(),
                ))
            })?;

        // Get customer's existing payment provider connections
        let connections = context.get_customer_connection_for_customer(customer);

        // Process the payment setup based on the selected strategy
        match strategy {
            SubscriptionPaymentStrategy::Auto => {
                self.setup_auto_payment(conn, customer, invoicing_entity_providers, connections)
                    .await
            }
            SubscriptionPaymentStrategy::Bank => {
                self.setup_bank_payment(customer, invoicing_entity_providers)
            }
            SubscriptionPaymentStrategy::External => Ok(PaymentSetupResult::external()),
        }
    }

    async fn use_or_create_connection(
        &self,
        conn: &mut PgConn,
        config: &Connector,
        customer: &Customer,
        customer_connectors: &mut Vec<CustomerConnection>,
    ) -> StoreResult<Option<CustomerConnectionId>> {
        if config.connector_type == ConnectorTypeEnum::PaymentProvider {
            // Find an existing connection to this provider
            let customer_connector_opt = customer_connectors
                .iter()
                .find(|cc| cc.connector_id == config.id && cc.customer_id == customer.id);

            let customer_connection_id = match customer_connector_opt {
                None => {
                    // Create a new customer in the payment provider
                    let provider = initialize_payment_provider(config)
                        .change_context(StoreError::PaymentProviderError)?;
                    let external_id = provider
                        .create_customer_in_provider(customer, config)
                        .await
                        .change_context(StoreError::PaymentProviderError)?;


                    let supported_payment_types = vec![];

                    // Connect the customer to the payment provider in our system
                    let connection_id = self
                        .connect_customer_payment_provider(
                            conn,
                            &customer.id,
                            &config.id,
                            &external_id,
                            supported_payment_types.clone()
                        )
                        .await?;

                    customer_connectors.push(CustomerConnection {
                        id: connection_id,
                        customer_id: customer.id,
                        connector_id: config.id,
                        supported_payment_types: Some(supported_payment_types),
                        external_customer_id: external_id,
                    });

                    connection_id
                }
                Some(cc) => cc.id,
            };
            Ok(Some(customer_connection_id))
        } else {
            Ok(None)
        }
    }

    /// Implements the Auto payment strategy with the following priority:
    /// 1. Use customer's default payment method if available
    /// 2. Use an existing customer connection to a payment provider
    /// 3. Create a new customer connection to the invoicing entity's payment provider
    /// 4. Fall back to associating a bank if available
    /// 5. Otherwise, use external payment
    ///
    /// TODO: Allow payment method selection by ID or type during subscription creation
    /// TODO: Add support for plan-specific payment method restrictions
    async fn setup_auto_payment(
        &self,
        conn: &mut PgConn,
        customer: &Customer,
        invoicing_entity_providers: &InvoicingEntityProviderSensitive,
        customer_connectors: Vec<&CustomerConnection>,
    ) -> StoreResult<PaymentSetupResult> {
        // Use customer's default payment method if available
        if let Some(payment_method) = &customer.current_payment_method_id {
            return Ok(PaymentSetupResult::with_existing_method(*payment_method));
        }

        // TODO support customer overrides  customer.card_provider_id + customer.direct_debit_provider_id

        // Check if customer has a default payment service provider connection
        // if let Some(card_provider_id) = &customer.card_provider_id { ... }
        // if let Some(card_provider_id) = &customer.direct_debit_provider_id { ... }

        // Try to use or create a connection to the invoicing entity's payment provider

        let mut connections: Vec<CustomerConnection> =
            customer_connectors.into_iter().cloned().collect();

        let card_connection = if let Some(card_provider) = &invoicing_entity_providers.card_provider
        {
            self.use_or_create_connection(conn, card_provider, customer, &mut connections)
                .await
        } else {
            Ok(None)
        }?;

        let direct_debit_connection = if let Some(direct_debit_provider) =
            &invoicing_entity_providers.direct_debit_provider
        {
            self.use_or_create_connection(conn, direct_debit_provider, customer, &mut connections)
                .await
        } else {
            Ok(None)
        }?;

        if card_connection.is_some() || direct_debit_connection.is_some() {
            return Ok(PaymentSetupResult::with_checkout(
                card_connection,
                direct_debit_connection,
            )); // TODO
        }

        // fallback on bank or external
        self.setup_bank_payment(customer, invoicing_entity_providers)
    }

    /// Sets up a bank transfer option using the invoicing entity's bank account
    /// TODO: Allow allow passing one as parameter during subscription creation
    fn setup_bank_payment(
        &self,
        customer: &Customer,
        invoicing_entity: &InvoicingEntityProviderSensitive,
    ) -> StoreResult<PaymentSetupResult> {
        if let Some(bank_account_id) = &customer.bank_account_id {
            Ok(PaymentSetupResult::with_bank(*bank_account_id))
        } else if let Some(bank_account) = &invoicing_entity.bank_account {
            Ok(PaymentSetupResult::with_bank(bank_account.id))
        } else {
            Ok(PaymentSetupResult::external())
        }
    }

    /// Creates a connection between a customer and a payment provider
    ///
    /// This stores the external customer ID from the provider in our system
    ///
    /// TODO: Support configurable payment method types beyond just Card
    async fn connect_customer_payment_provider(
        &self,
        conn: &mut PgConn,
        customer_id: &CustomerId,
        connector_id: &ConnectorId,
        external_id: &str,
        supported_payment_method_types: Vec<PaymentMethodTypeEnum>
    ) -> StoreResult<CustomerConnectionId> {
        let customer_connection: CustomerConnectionRow = CustomerConnection {
            id: CustomerConnectionId::new(),
            external_customer_id: external_id.to_string(),
            customer_id: *customer_id,
            connector_id: *connector_id,
            supported_payment_types: Some(supported_payment_method_types),
        }
        .into();

        let res = customer_connection
            .insert(conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(res.id)
    }
}

/// Represents the result of payment setup process
///
/// This structure contains all the information needed for handling payments
/// for a subscription based on the determined payment strategy.
#[derive(Debug, Clone)]
pub struct PaymentSetupResult {
    /// The customer's connection to a payment provider, if applicable
    pub card_connection_id: Option<CustomerConnectionId>,
    pub direct_debit_connection_id: Option<CustomerConnectionId>,

    /// Indicates whether a checkout session is needed to collect payment details
    pub checkout: bool,

    /// An existing payment method to use for the subscription
    pub payment_method: Option<CustomerPaymentMethodId>,

    /// A bank account to use for direct transfers
    pub bank: Option<BankAccountId>,
}

impl PaymentSetupResult {
    /// Creates a payment setup result for initiating a checkout flow
    fn with_checkout(
        card_connection_id: Option<CustomerConnectionId>,
        direct_debit_connection_id: Option<CustomerConnectionId>,
    ) -> Self {
        Self {
            card_connection_id,
            direct_debit_connection_id,
            checkout: true,
            payment_method: None,
            bank: None,
        }
    }

    /// Creates a payment setup result using an existing payment method
    fn with_existing_method(method_id: CustomerPaymentMethodId) -> Self {
        Self {
            card_connection_id: None,
            direct_debit_connection_id: None,
            checkout: false,
            bank: None,
            payment_method: Some(method_id),
        }
    }

    /// Creates a payment setup result associating a bank account for direct transfers
    fn with_bank(bank_account_id: BankAccountId) -> Self {
        Self {
            card_connection_id: None,
            direct_debit_connection_id: None,
            checkout: false,
            bank: Some(bank_account_id),
            payment_method: None,
        }
    }

    /// Creates a payment setup result for external/manual payments
    fn external() -> Self {
        Self {
            card_connection_id: None,
            direct_debit_connection_id: None,
            checkout: false,
            payment_method: None,
            bank: None,
        }
    }
}
