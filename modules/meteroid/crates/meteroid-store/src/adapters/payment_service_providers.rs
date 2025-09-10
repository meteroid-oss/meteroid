use crate::domain::connectors::{Connector, ProviderData, ProviderSensitiveData};
use crate::domain::customer_payment_methods::SetupIntent;
use crate::domain::enums::ConnectorProviderEnum;
use crate::domain::payment_transactions::PaymentIntent;
use crate::domain::{Address, Customer, CustomerConnection, PaymentMethodTypeEnum};
use crate::utils::local_id::LocalId;
use async_trait::async_trait;
use common_domain::ids::{BaseId, PaymentTransactionId, TenantId};
use diesel_models::enums::PaymentStatusEnum;
use error_stack::{Report, ResultExt, bail};
use secrecy::SecretString;
use std::collections::HashMap;
use stripe_client::client::StripeClient;
use stripe_client::customers::{
    CreateCustomer, CustomerApi, CustomerShipping, OptionalFieldsAddress,
};
use stripe_client::payment_intents::{
    PaymentIntentApi, PaymentIntentRequest, StripePaymentIntent, StripePaymentStatus,
};
use stripe_client::payment_methods::PaymentMethodsApi;
use stripe_client::setup_intents::{
    CreateSetupIntent, CreateSetupIntentUsage, SetupIntentApi, StripePaymentMethodType,
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum PaymentProviderError {
    #[error("Provider configuration error: {0}")]
    Configuration(String),
    #[error("Customer creation failed: {0}")]
    CustomerCreation(String),
    #[error("Setup Intent error: {0}")]
    SetupIntent(String),
    #[error("Payment Intent error: {0}")]
    PaymentIntent(String),
    #[error("Database error: {0}")]
    Database(#[from] diesel::result::Error),
    #[error("Missing metadata: {0}")]
    MissingMetadata(String),
    #[error("Invalid metadata")]
    InvalidMetadata,
}

#[async_trait]
pub trait PaymentProvider: Send + Sync {
    async fn create_customer_in_provider(
        &self,
        customer: &Customer,
        connector: &Connector,
    ) -> error_stack::Result<String, PaymentProviderError>;
    async fn get_payment_method_from_provider(
        &self,
        connector: &Connector,
        payment_method_id: &str,
        customer_id: &str,
    ) -> error_stack::Result<stripe_client::payment_methods::PaymentMethod, PaymentProviderError>;
    async fn create_setup_intent_in_provider(
        &self,
        connection: &CustomerConnection,
        connector: &Connector,
        payment_methods: Vec<PaymentMethodTypeEnum>,
    ) -> error_stack::Result<SetupIntent, PaymentProviderError>;

    async fn create_payment_intent_in_provider(
        &self,
        connector: &Connector,
        transaction_id: &PaymentTransactionId,
        customer_external_id: &str,
        payment_method_external_id: &str,
        amount: i64,
        currency: &str,
    ) -> error_stack::Result<PaymentIntent, PaymentProviderError>;
}

pub fn initialize_payment_provider(
    config: &Connector,
) -> error_stack::Result<Box<dyn PaymentProvider>, PaymentProviderError> {
    match config.provider {
        ConnectorProviderEnum::Stripe => Ok(Box::new(StripeClient::new())),
        _ => bail!(PaymentProviderError::Configuration(
            "unknown payment provider".to_owned()
        )),
    }
}

#[async_trait::async_trait]
impl PaymentProvider for StripeClient {
    async fn create_customer_in_provider(
        &self,
        customer: &Customer,
        connector: &Connector,
    ) -> error_stack::Result<String, PaymentProviderError> {
        let secret_key = extract_stripe_secret_key(connector)?;
        fn map_address(a: &Address) -> OptionalFieldsAddress {
            OptionalFieldsAddress {
                city: a.city.clone(),
                country: a.country.clone(),
                line1: a.line1.clone(),
                line2: a.line2.clone(),
                state: a.state.clone(),
                postal_code: a.zip_code.clone(),
            }
        }

        // add instance (org, tenant slug ?)
        let mut metadata = HashMap::from([
            ("meteroid.id".to_string(), customer.id.as_base62()),
            (
                "meteroid.tenant_id".to_string(),
                customer.tenant_id.as_base62(),
            ),
        ]);

        if let Some(alias) = &customer.alias {
            metadata.insert("meteroid.alias".to_string(), alias.clone().to_string());
        }

        let res = self
            .create_customer(
                CreateCustomer {
                    name: Some(customer.name.clone()),
                    address: customer.billing_address.as_ref().map(map_address),
                    email: customer.billing_email.clone(),
                    source: None, // drop, not what I expected
                    shipping: customer
                        .shipping_address
                        .as_ref()
                        .and_then(|a| a.address.as_ref())
                        .map(|a| CustomerShipping {
                            address: map_address(a),
                            name: customer.name.clone(),
                            phone: customer.phone.clone(),
                        }),
                    metadata: Some(metadata),
                    phone: customer.phone.clone(),
                    description: None,
                    preferred_locales: None,
                    validate: None,
                    coupon: None,
                },
                &secret_key,
                LocalId::no_prefix(), //customer.local_id.clone(),
            )
            .await
            .map_err(|e| PaymentProviderError::CustomerCreation(e.to_string()))?;

        Ok(res.id)
    }

    async fn get_payment_method_from_provider(
        &self,
        connector: &Connector,
        payment_method_id: &str,
        customer_id: &str,
    ) -> error_stack::Result<stripe_client::payment_methods::PaymentMethod, PaymentProviderError>
    {
        let secret_key = extract_stripe_secret_key(connector)?;

        self.get_payment_method(payment_method_id, customer_id, &secret_key)
            .await
            .map_err(|e| Report::new(PaymentProviderError::Configuration(e.to_string())))
    }

    async fn create_setup_intent_in_provider(
        &self,
        connection: &CustomerConnection,
        connector: &Connector,
        payment_methods: Vec<PaymentMethodTypeEnum>,
    ) -> error_stack::Result<SetupIntent, PaymentProviderError> {
        let secret_key = extract_stripe_secret_key(connector)?;
        let public_key = extract_stripe_public_key(connector)?;

        let stripe_payment_methods = payment_methods
            .into_iter()
            .filter_map(|method| match method {
                PaymentMethodTypeEnum::Card => Some(StripePaymentMethodType::Card),
                PaymentMethodTypeEnum::DirectDebitSepa => Some(StripePaymentMethodType::Sepa),
                PaymentMethodTypeEnum::DirectDebitAch => Some(StripePaymentMethodType::Ach),
                PaymentMethodTypeEnum::DirectDebitBacs => Some(StripePaymentMethodType::Bacs),
                PaymentMethodTypeEnum::Other => None,
                PaymentMethodTypeEnum::Transfer => None,
            })
            .collect();

        let metadata = HashMap::from([
            (
                "meteroid.tenant_id".to_string(),
                connector.tenant_id.as_base62(),
            ),
            (
                "meteroid.customer_id".to_string(),
                connection.customer_id.as_base62(),
            ),
            (
                "meteroid.connection_id".to_string(),
                connection.id.as_base62(),
            ),
        ]);

        let setup_intent = self
            .create_setup_intent(
                CreateSetupIntent {
                    customer: Some(connection.external_customer_id.clone()),
                    payment_method_types: Some(stripe_payment_methods),
                    usage: Some(CreateSetupIntentUsage::OffSession),
                    setup_mandate_details: None,
                    metadata,
                },
                &secret_key,
                Uuid::now_v7().to_string(), // TODO pass idempotency from api (though we already do check idp at the api level)
            )
            .await
            .map_err(|e| PaymentProviderError::SetupIntent(e.to_string()))?;

        Ok(SetupIntent {
            intent_id: setup_intent.id,
            client_secret: setup_intent.client_secret,
            public_key,
            provider: ConnectorProviderEnum::Stripe,
            connector_id: connector.id,
            connection_id: connection.id,
        })
    }

    async fn create_payment_intent_in_provider(
        &self,
        connector: &Connector,
        transaction_id: &PaymentTransactionId,
        customer_external_id: &str,
        payment_method_external_id: &str,
        amount: i64,
        currency: &str,
    ) -> error_stack::Result<PaymentIntent, PaymentProviderError> {
        let secret_key = extract_stripe_secret_key(connector)?;

        let metadata = HashMap::from([
            (
                "meteroid.tenant_id".to_string(),
                connector.tenant_id.as_base62(),
            ),
            (
                "meteroid.transaction_id".to_string(),
                transaction_id.as_base62(),
            ),
        ]);

        let payment_intent = self
            .create_payment_intent(
                PaymentIntentRequest {
                    amount,
                    currency: currency.to_string(),
                    customer: Some(customer_external_id.to_string()),
                    setup_mandate_details: None,
                    payment_method: payment_method_external_id.to_string(),
                    confirm: true,
                    metadata,
                    off_session: Some(true),
                    return_url: None,
                    capture_method: Default::default(),
                },
                &secret_key,
                Uuid::now_v7().to_string(), // TODO pass idempotency from api ?
            )
            .await
            .map_err(|e| PaymentProviderError::PaymentIntent(e.to_string()))?;

        Ok(payment_intent.try_into()?)
    }
}

impl TryFrom<StripePaymentIntent> for PaymentIntent {
    type Error = Report<PaymentProviderError>;

    fn try_from(intent: StripePaymentIntent) -> Result<Self, Self::Error> {
        let tenant_id = intent
            .metadata
            .get("meteroid.tenant_id")
            // TODO search :  .get("customer_id")
            .ok_or(PaymentProviderError::MissingMetadata(
                "meteroid.tenant_id".to_string(),
            ))?;
        let tenant_id = TenantId::parse_base62(tenant_id)
            .change_context(PaymentProviderError::InvalidMetadata)?;

        let transaction_id = intent.metadata.get("meteroid.transaction_id").ok_or(
            PaymentProviderError::MissingMetadata("meteroid.transaction_id".to_string()),
        )?;
        let transaction_id = PaymentTransactionId::parse_base62(transaction_id)
            .change_context(PaymentProviderError::InvalidMetadata)?;

        let (new_status, processed_at) = match intent.status {
            StripePaymentStatus::Succeeded => (
                PaymentStatusEnum::Settled,
                Some(chrono::Utc::now().naive_utc()),
            ),
            StripePaymentStatus::Failed => (PaymentStatusEnum::Failed, None),
            StripePaymentStatus::Canceled => (PaymentStatusEnum::Cancelled, None),
            StripePaymentStatus::Pending | StripePaymentStatus::Processing => {
                (PaymentStatusEnum::Pending, None)
            }
            StripePaymentStatus::RequiresCustomerAction
            | StripePaymentStatus::RequiresPaymentMethod
            | StripePaymentStatus::RequiresConfirmation
            | StripePaymentStatus::RequiresCapture => {
                // Customer action is required - keep as Pending but we might want to notify the customer
                tracing::log::info!(
                    "Payment intent {} requires customer action: {:?}",
                    intent.id,
                    intent.status
                );
                (PaymentStatusEnum::Pending, None)
            }
            StripePaymentStatus::Chargeable | StripePaymentStatus::Consumed => {
                tracing::log::warn!(
                    "Unhandled stripe payment status for transaction {}: {:?}",
                    intent.id,
                    intent.status
                );
                return Err(Report::new(PaymentProviderError::PaymentIntent(format!(
                    "Unhandled payment status: {:?}",
                    intent.status
                ))));
            }
        };

        Ok(PaymentIntent {
            external_id: intent.id,
            amount_requested: intent.amount,
            amount_received: intent.amount_received,
            currency: intent.currency,
            next_action: intent.next_action,
            status: new_status.into(),
            processed_at,
            last_payment_error: intent.last_payment_error,
            tenant_id,
            transaction_id,
        })
    }
}

fn extract_stripe_secret_key(
    connector: &Connector,
) -> error_stack::Result<SecretString, PaymentProviderError> {
    match &connector.sensitive {
        Some(ProviderSensitiveData::Stripe(data)) => {
            Ok(SecretString::new(data.api_secret_key.clone()))
        }
        Some(_) => Err(Report::new(PaymentProviderError::Configuration(
            "Not a stripe connector".to_string(),
        ))),
        None => Err(Report::new(PaymentProviderError::Configuration(
            "No api_secret_key found".to_string(),
        ))),
    }
}

fn extract_stripe_public_key(
    connector: &Connector,
) -> error_stack::Result<SecretString, PaymentProviderError> {
    match &connector.data {
        Some(ProviderData::Stripe(data)) => Ok(SecretString::new(data.api_publishable_key.clone())),
        Some(_) => Err(Report::new(PaymentProviderError::Configuration(
            "not a stripe connection".to_string(),
        ))),
        None => Err(Report::new(PaymentProviderError::Configuration(
            "No api_publishable_key found".to_string(),
        ))),
    }
}
