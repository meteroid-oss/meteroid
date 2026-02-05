use crate::domain::enums::PlanTypeEnum;
use crate::domain::subscriptions::OnlineMethodsConfig;
use crate::domain::{
    Customer, InvoicingEntityProviderSensitive, PaymentMethodsConfig,
    SubscriptionActivationCondition, SubscriptionNew,
};
use crate::errors::StoreError;

use super::context::SubscriptionCreationContext;
use crate::{StoreResult, services::Services};
use error_stack::Report;

impl Services {
    /// Validates payment configuration and determines if checkout is needed.
    /// Connection creation happens on-demand at checkout time.
    pub(crate) fn setup_payment_provider(
        &self,
        subscription: &SubscriptionNew,
        customer: &Customer,
        context: &SubscriptionCreationContext,
    ) -> StoreResult<PaymentSetupResult> {
        let plan = context
            .plans
            .iter()
            .find(|p| p.version_id == subscription.plan_version_id)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(
                    "No plan found for subscription".to_string(),
                ))
            })?;

        if plan.plan_type == PlanTypeEnum::Free {
            return Ok(PaymentSetupResult::external());
        }

        let invoicing_entity_providers = context
            .get_invoicing_entity_providers_for_customer(customer)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(
                    "No invoicing entity found for customer".to_string(),
                ))
            })?;

        let config = subscription
            .payment_methods_config
            .clone()
            .unwrap_or_else(PaymentMethodsConfig::online);

        let has_online_capability = match &config {
            PaymentMethodsConfig::Online { config } => {
                self.has_online_capability(invoicing_entity_providers, config.as_ref())
            }
            PaymentMethodsConfig::BankTransfer { .. } | PaymentMethodsConfig::External => false,
        };

        if matches!(
            subscription.activation_condition,
            SubscriptionActivationCondition::OnCheckout
        ) && !has_online_capability
        {
            return Err(Report::new(StoreError::InvalidArgument(
                "OnCheckout activation requires card or direct debit to be enabled. Configure a payment provider on the invoicing entity and ensure the subscription uses Online payment config.".to_string(),
            )));
        }

        if subscription.charge_automatically && !has_online_capability {
            return Err(Report::new(StoreError::InvalidArgument(
                "Automatic charging requires card or direct debit to be enabled. Configure a payment provider or set charge_automatically to false.".to_string(),
            )));
        }

        match &config {
            PaymentMethodsConfig::Online { config } => self.setup_online_payment(
                invoicing_entity_providers,
                config.as_ref(),
                subscription.activation_condition.clone(),
            ),
            PaymentMethodsConfig::BankTransfer { account_id } => {
                self.setup_bank_transfer_payment(invoicing_entity_providers, *account_id)
            }
            PaymentMethodsConfig::External => Ok(PaymentSetupResult::external()),
        }
    }

    fn has_online_capability(
        &self,
        invoicing_entity_providers: &InvoicingEntityProviderSensitive,
        online_config: Option<&OnlineMethodsConfig>,
    ) -> bool {
        let card_enabled = online_config
            .and_then(|c| c.card.as_ref())
            .map(|m| m.enabled)
            .unwrap_or(true);

        let direct_debit_enabled = online_config
            .and_then(|c| c.direct_debit.as_ref())
            .map(|m| m.enabled)
            .unwrap_or(true);

        (card_enabled && invoicing_entity_providers.card_provider.is_some())
            || (direct_debit_enabled && invoicing_entity_providers.direct_debit_provider.is_some())
    }

    fn setup_online_payment(
        &self,
        invoicing_entity_providers: &InvoicingEntityProviderSensitive,
        online_config: Option<&OnlineMethodsConfig>,
        condition: SubscriptionActivationCondition,
    ) -> StoreResult<PaymentSetupResult> {
        let checkout = matches!(condition, SubscriptionActivationCondition::OnCheckout);

        if self.has_online_capability(invoicing_entity_providers, online_config) {
            return Ok(PaymentSetupResult { checkout });
        }

        Ok(PaymentSetupResult::external())
    }

    fn setup_bank_transfer_payment(
        &self,
        invoicing_entity_providers: &InvoicingEntityProviderSensitive,
        account_id_override: Option<common_domain::ids::BankAccountId>,
    ) -> StoreResult<PaymentSetupResult> {
        let has_bank_account =
            account_id_override.is_some() || invoicing_entity_providers.bank_account.is_some();

        if has_bank_account {
            return Ok(PaymentSetupResult { checkout: false });
        }

        Ok(PaymentSetupResult::external())
    }
}

#[derive(Debug, Clone)]
pub struct PaymentSetupResult {
    pub checkout: bool,
}

impl PaymentSetupResult {
    pub(crate) fn external() -> Self {
        Self { checkout: false }
    }
}
