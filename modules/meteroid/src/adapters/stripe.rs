use error_stack::Report;
use hyper::StatusCode;
use secrecy::ExposeSecret;
use secrecy::SecretString;
use std::sync::Arc;
use stripe_client::webhook::Event;
use stripe_client::webhook::EventObject;

use crate::errors;

use super::types::{AdapterCommon, WebhookAdapter};
use crate::adapters::types::ParsedRequest;
use axum::response::IntoResponse;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerConnectionId, CustomerPaymentMethodId};
use error_stack::ResultExt;
use meteroid_store::Store;
use meteroid_store::domain::{CustomerPaymentMethodNew, PaymentIntent};
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use scoped_futures::ScopedFutureExt;
use stripe_client::setup_intents::SetupIntent;
use stripe_client::webhook::StripeWebhook;
use stripe_client::webhook::event_type;

use meteroid_store::adapters::payment_service_providers::{PaymentProvider, PaymentProviderError};
use meteroid_store::domain::connectors::Connector;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

static STRIPE: std::sync::OnceLock<Stripe> = std::sync::OnceLock::new();

#[derive(Debug, Clone)]
pub struct Stripe {
    pub client: Arc<stripe_client::client::StripeClient>,
}

impl AdapterCommon for Stripe {
    fn id(&self) -> &'static str {
        "stripe"
    }
}

#[async_trait::async_trait]
impl WebhookAdapter for Stripe {
    async fn verify_webhook(
        &self,
        request: &ParsedRequest,
        security: &SecretString,
    ) -> Result<bool, Report<errors::AdapterWebhookError>> {
        let sig = request
            .headers
            .get("Stripe-Signature")
            .map(|header_value| {
                header_value
                    .to_str()
                    .map(String::from)
                    .map_err(|_| errors::AdapterWebhookError::SignatureNotFound)
                    .map_err(Report::from)
            })
            .ok_or(errors::AdapterWebhookError::SignatureNotFound)
            .map_err(Report::from)??;

        let secret = security.expose_secret();

        // hmac accepts an empty key, which would make every forged signature verifiable.
        if secret.is_empty() {
            return Err(
                Report::new(errors::AdapterWebhookError::SignatureVerificationFailed)
                    .attach("connector has no webhook secret configured"),
            );
        }

        StripeWebhook::validate_signature(
            &String::from_utf8_lossy(&request.raw_body),
            &sig,
            secret,
        )
        .change_context(errors::AdapterWebhookError::SignatureVerificationFailed)?;
        Ok(true)
    }

    fn get_optimistic_webhook_response(&self) -> axum::response::Response {
        (StatusCode::OK, "OK").into_response()
    }

    async fn process_webhook_event(
        &self,
        request: &ParsedRequest,
        connector: &Connector,
        store: Store,
    ) -> Result<bool, Report<errors::AdapterWebhookError>> {
        log::info!(
            "Processing webhook Event: {:?}",
            request.json_body.to_string().as_str()
        );

        let parsed = StripeWebhook::parse_event(request.json_body.to_string().as_str())
            .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

        let object = parsed.data.object.clone();

        match object {
            EventObject::SetupIntent(data) => {
                self.process_setup_intent_events(parsed, data, connector, store)
                    .await
            }
            EventObject::PaymentIntent(data) => {
                let payment_intent: Result<PaymentIntent, Report<PaymentProviderError>> =
                    data.try_into();
                let payment_intent =
                    payment_intent.change_context(errors::AdapterWebhookError::ProviderError)?;
                self.process_payment_intent_events(parsed, payment_intent, connector, store)
                    .await
            }
        }?;

        Ok(true)
    }
}

impl Stripe {
    pub fn get() -> &'static Self {
        STRIPE.get_or_init(|| Stripe {
            client: Arc::new(stripe_client::client::StripeClient::new()),
        })
    }

    async fn process_setup_intent_events(
        &self,
        parsed: Event,
        data: SetupIntent,
        connector: &Connector,
        store: Store,
    ) -> Result<bool, Report<errors::AdapterWebhookError>> {
        let event_type_clone = parsed.event_type.clone();

        if event_type_clone != event_type::SETUP_INTENT_SUCCEEDED {
            log::info!("Ignoring webhook event type: {event_type_clone}");
            return Ok(false);
        }

        log::info!("Processing webhook event type: {event_type_clone}");

        let connection_id = data.metadata.get("meteroid.connection_id").ok_or(
            errors::AdapterWebhookError::MissingMetadata("meteroid.connection_id".to_string()),
        )?;

        let connection_id = CustomerConnectionId::parse_base62(connection_id)
            .change_context(errors::AdapterWebhookError::InvalidMetadata)?;

        // Resolve the connection under the signing connector's tenant and take the customer from
        // that row. The metadata is provider-supplied, so trusting its customer id would let one
        // tenant attach a payment method against another tenant's customer.
        let connection = store
            .get_connection_by_id(&connector.tenant_id, &connection_id)
            .await
            .change_context(errors::AdapterWebhookError::InvalidMetadata)?;

        if connection.connector_id != connector.id {
            return Err(Report::new(errors::AdapterWebhookError::InvalidMetadata)
                .attach("setup intent names a connection belonging to another connector"));
        }

        let customer_id = connection.customer_id;

        let payment_method =
            data.payment_method
                .ok_or(errors::AdapterWebhookError::MissingMetadata(
                    "payment_method".to_string(),
                ))?;

        let customer = data
            .customer
            .ok_or(errors::AdapterWebhookError::MissingMetadata(
                "customer".to_string(),
            ))?;

        let method = self
            .client
            .get_payment_method_from_provider(connector, &payment_method, &customer)
            .await
            .change_context(errors::AdapterWebhookError::ProviderError)?;

        let payment_method = store
            .upsert_payment_method(CustomerPaymentMethodNew {
                id: CustomerPaymentMethodId::new(),
                tenant_id: connector.tenant_id,
                customer_id,
                connection_id,
                external_payment_method_id: method.external_payment_method_id,
                payment_method_type: method.payment_method_type,
                account_number_hint: method.account_number_hint,
                card_brand: method.card_brand,
                card_last4: method.card_last4,
                card_exp_month: method.card_exp_month,
                card_exp_year: method.card_exp_year,
            })
            .await
            .change_context(errors::AdapterWebhookError::StoreError)?;

        // Set as default payment method for the customer
        use meteroid_store::domain::CustomerPatch;
        let customer_patch = CustomerPatch {
            id: customer_id,
            name: None,
            alias: None,
            billing_email: None,
            phone: None,
            balance_value_cents: None,
            currency: None,
            billing_address: None,
            shipping_address: None,
            invoicing_entity_id: None,
            vat_number: None,
            current_payment_method_id: Some(Some(payment_method.id)),
            invoicing_emails: None,
            is_tax_exempt: None,
            custom_taxes: None,
            connected_account_id: None,
        };

        store
            .patch_customer(Actor::System, connector.tenant_id, customer_patch)
            .await
            .change_context(errors::AdapterWebhookError::StoreError)?;

        Ok(true)
    }

    async fn process_payment_intent_events(
        &self,
        parsed: Event,
        data: PaymentIntent,
        connector: &Connector,
        store: Store,
    ) -> Result<bool, Report<errors::AdapterWebhookError>> {
        let event_type_clone = parsed.event_type.clone();

        // TODO the partially funded case
        if event_type_clone != event_type::PAYMENT_INTENT_SUCCEEDED
            && event_type_clone != event_type::PAYMENT_INTENT_FAILED
            && event_type_clone != event_type::PAYMENT_INTENT_PARTIALLY_FUNDED
        {
            log::info!("Ignoring webhook event type: {event_type_clone}");
            return Ok(false);
        }

        log::info!("Processing webhook event type: {event_type_clone}");

        // The tenant here comes from provider metadata, which is only as trustworthy as the
        // connector whose secret signed this event. Without this check any tenant could settle
        // another tenant's transaction by naming it in a webhook signed with their own secret.
        if data.tenant_id != connector.tenant_id {
            return Err(Report::new(errors::AdapterWebhookError::InvalidMetadata)
                .attach("payment intent names a tenant other than the signing connector's"));
        }

        let tenant_id = connector.tenant_id;

        // we fetch the related transaction then we consolidate the transaction with the new intent event
        store
            .transaction(|conn| {
                let store = store.clone();
                async move {
                    let inserted_transaction = store
                        .get_payment_tx_by_id_for_update(conn, data.transaction_id, tenant_id)
                        .await?;

                    // The provider reports its own amount and currency. A settlement that does
                    // not match what Meteroid asked for must not close out the transaction.
                    if inserted_transaction.amount != data.amount_requested
                        || !inserted_transaction
                            .currency
                            .eq_ignore_ascii_case(&data.currency)
                    {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "payment intent does not match transaction {}: expected {} {}, got {} {}",
                            inserted_transaction.id,
                            inserted_transaction.amount,
                            inserted_transaction.currency,
                            data.amount_requested,
                            data.currency,
                        ))));
                    }

                    store
                        .consolidate_intent_and_transaction_tx(
                            conn,
                            &Actor::System,
                            inserted_transaction,
                            data,
                        )
                        .await?;

                    Ok(())
                }
                .scope_boxed()
            })
            .await
            .change_context(errors::AdapterWebhookError::StoreError)?;

        Ok(true)
    }
}
