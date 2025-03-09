use error_stack::Report;
use error_stack::Result;
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
use error_stack::ResultExt;
use meteroid_store::Store;
use stripe_client::payment_intents::PaymentIntent;
use stripe_client::setup_intents::SetupIntent;
use stripe_client::webhook::StripeWebhook;
use stripe_client::webhook::event_type;

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
    ) -> Result<bool, errors::AdapterWebhookError> {
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
        store: Store,
    ) -> Result<bool, errors::AdapterWebhookError> {
        log::info!(
            "Processing webhook Event: {:?}",
            request.json_body.to_string().as_str()
        );

        let parsed = StripeWebhook::parse_event(request.json_body.to_string().as_str())
            .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

        let object = parsed.data.object.clone();

        match object {
            EventObject::SetupIntent(data) => {
                self.process_setup_intent_events(parsed, data, store).await
            }
            EventObject::PaymentIntent(data) => {
                self.process_payment_intent_events(parsed, data, store)
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
        _store: Store,
    ) -> Result<bool, errors::AdapterWebhookError> {
        let event_type_clone = parsed.event_type.clone();

        if event_type_clone != event_type::SETUP_INTENT_SUCCEEDED {
            log::info!("Ignoring webhook event type: {}", event_type_clone);
            return Ok(false);
        }

        log::info!("Processing webhook event type: {}", event_type_clone);
        log::info!("Event: {:?}", data);

        // store.upsert_payment_method

        Ok(true)
    }

    async fn process_payment_intent_events(
        &self,
        parsed: Event,
        data: PaymentIntent,
        _store: Store,
    ) -> Result<bool, errors::AdapterWebhookError> {
        let event_type_clone = parsed.event_type.clone();

        // TODO the partially funded case
        if event_type_clone != event_type::PAYMENT_INTENT_SUCCEEDED
            || event_type_clone != event_type::PAYMENT_INTENT_FAILED
            || event_type_clone != event_type::PAYMENT_INTENT_PARTIALLY_FUNDED
        {
            log::info!("Ignoring webhook event type: {}", event_type_clone);
            return Ok(false);
        }

        log::info!("Processing webhook event type: {}", event_type_clone);
        log::info!("Event: {:?}", data);

        // store.consolidate_transaction

        Ok(true)
    }
}
