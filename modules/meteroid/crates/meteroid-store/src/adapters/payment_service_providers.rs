use crate::domain::connectors::{Connector, ProviderData, ProviderSensitiveData};
use crate::domain::customer_payment_methods::SetupIntent;
use crate::domain::enums::ConnectorProviderEnum;
use crate::domain::{Address, Customer};
use crate::utils::local_id::LocalId;
use async_trait::async_trait;
use error_stack::Report;
use secrecy::SecretString;
use std::collections::HashMap;
use stripe_client::client::StripeClient;
use stripe_client::customers::{
    CreateCustomer, CustomerApi, CustomerShipping, OptionalFieldsAddress,
};
use stripe_client::payment_intents::{
    FutureUsage, PaymentIntent, PaymentIntentApi, PaymentIntentRequest,
};
use stripe_client::setup_intents::{CreateSetupIntent, CreateSetupIntentUsage, SetupIntentApi};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum PaymentProviderError {
    #[error("Provider configuration error: {0}")]
    Configuration(String),
    #[error("Provider communication failed: {0}")]
    ProviderError(String),
    #[error("Customer creation failed: {0}")]
    CustomerCreation(String),
    #[error("Invalid payment method: {0}")]
    PaymentMethod(String),
    #[error("Setup Intent error: {0}")]
    SetupIntent(String),
    #[error("Payment Intent error: {0}")]
    PaymentIntent(String),
    #[error("Database error: {0}")]
    Database(#[from] diesel::result::Error),
    #[error("Validation error: {0}")]
    Validation(String),
}

#[async_trait]
pub trait PaymentProvider: Send + Sync {
    async fn create_customer_in_provider(
        &self,
        customer: &Customer,
        connector: &Connector,
    ) -> error_stack::Result<String, PaymentProviderError>;
    async fn create_setup_intent_in_provider(
        &self,
        connector: &Connector,
        customer_external_id: &String,
    ) -> error_stack::Result<SetupIntent, PaymentProviderError>;

    async fn create_payment_intent_in_provider(
        &self,
        connector: &Connector,
        customer_external_id: &String,
        payment_method_external_id: &String,
        amount: i64,
        currency: &str,
    ) -> error_stack::Result<PaymentIntent, PaymentProviderError>;
}

pub fn initialize_payment_provider(config: &Connector) -> Box<dyn PaymentProvider> {
    match config.provider {
        ConnectorProviderEnum::Stripe => Box::new(StripeClient::new()),
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
        };

        // add instance (org, tenant slug ?)
        let mut metadata = HashMap::from([
            ("meteroid.id".to_string(), customer.id.clone().to_string()),
            (
                "meteroid.tenant_id".to_string(),
                customer.tenant_id.clone().to_string(),
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

    async fn create_setup_intent_in_provider(
        &self,
        connector: &Connector,
        customer_external_id: &String,
    ) -> error_stack::Result<SetupIntent, PaymentProviderError> {
        let secret_key = extract_stripe_secret_key(connector)?;
        let public_key = extract_stripe_public_key(connector)?;

        let setup_intent = self
            .create_setup_intent(
                CreateSetupIntent {
                    customer: Some(customer_external_id.clone()),
                    payment_method_types: Some(vec!["card".to_string()]), // TODO
                    usage: Some(CreateSetupIntentUsage::OffSession),
                    setup_mandate_details: None, // TODO double check
                },
                &secret_key,
                Uuid::now_v7().to_string(), // TODO pass idempotency from api ?
            )
            .await
            .map_err(|e| PaymentProviderError::SetupIntent(e.to_string()))?;

        Ok(SetupIntent {
            intent_id: setup_intent.id,
            client_secret: setup_intent.client_secret,
            public_key,
            cc_provider: "stripe".to_string(),
            cc_provider_id: connector.id,
        })
    }

    async fn create_payment_intent_in_provider(
        &self,
        connector: &Connector,
        customer_external_id: &String,
        payment_method_external_id: &String,
        amount: i64,
        currency: &str,
    ) -> error_stack::Result<PaymentIntent, PaymentProviderError> {
        let secret_key = extract_stripe_secret_key(connector)?;

        let metadata = HashMap::from([(
            "meteroid.tenant_id".to_string(),
            connector.tenant_id.clone().to_string(),
        )]);

        let payment_intent = self
            .create_payment_intent(
                PaymentIntentRequest {
                    amount,
                    currency: currency.to_string(),
                    customer: Some(customer_external_id.clone()),
                    setup_mandate_details: None,
                    payment_method: payment_method_external_id.clone(),
                    confirm: true,
                    metadata,
                    off_session: Some(true),
                    return_url: None,
                    setup_future_usage: FutureUsage::OffSession,
                    capture_method: Default::default(),
                },
                &secret_key,
                Uuid::now_v7().to_string(), // TODO pass idempotency from api ?
            )
            .await
            .map_err(|e| PaymentProviderError::PaymentIntent(e.to_string()))?;

        Ok(payment_intent)
    }
}

fn extract_stripe_secret_key(
    connector: &Connector,
) -> error_stack::Result<SecretString, PaymentProviderError> {
    match &connector.sensitive {
        Some(ProviderSensitiveData::Stripe(data)) => {
            Ok(SecretString::new(data.api_secret_key.clone()))
        }
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
        None => Err(Report::new(PaymentProviderError::Configuration(
            "No api_publishable_key found".to_string(),
        ))),
    }
}
