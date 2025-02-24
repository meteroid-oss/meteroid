use error_stack::bail;
use error_stack::Report;
use error_stack::Result;
use hyper::StatusCode;
use secrecy::ExposeSecret;
use secrecy::SecretString;
use std::sync::Arc;
use stripe_client::invoice::{CollectionMethod, CreateInvoice, MeteroidMetadata};
use stripe_client::invoice::{CreateInvoiceItem, Invoice, Period};
use stripe_client::webhook::Event;
use stripe_client::webhook::EventObject;

use crate::errors;

use super::types::{AdapterCommon, WebhookAdapter};
use crate::adapters::types::{InvoicingAdapter, ParsedRequest};
use crate::errors::InvoicingAdapterError;
use axum::response::IntoResponse;
use error_stack::ResultExt;
use meteroid_store::domain::enums::InvoiceExternalStatusEnum;
use meteroid_store::domain::{
    BillingConfig, Customer, LineItem, StripeCollectionMethod,
    StripeCustomerConfig as BillingConfigStripe,
};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::{domain, Store};
use stripe_client::webhook::event_type;
use stripe_client::webhook::StripeWebhook;
use uuid::Uuid;

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
        let parsed = StripeWebhook::parse_event(request.json_body.to_string().as_str())
            .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

        let object = parsed.data.object.clone();

        match object {
            EventObject::Invoice(invoice) => {
                self.process_invoice_events(parsed, invoice, store).await
            }
        }?;

        Ok(true)
    }
}

#[async_trait::async_trait]
impl InvoicingAdapter for Stripe {
    async fn send_invoice(
        &self,
        invoice: &domain::Invoice,
        customer: &Customer,
        api_key: SecretString,
    ) -> Result<(), InvoicingAdapterError> {
        let api_key = &api_key;

        let stripe_customer = Self::extract_stripe_customer_id(customer)?;
        let collection_method = Self::extract_stripe_collection_method(customer)?;

        let create_invoice =
            Self::db_invoice_to_external(invoice, &stripe_customer, collection_method);

        let created_stripe_invoice = self
            .client
            .create_invoice(create_invoice, api_key, invoice.id.to_string())
            .await
            .change_context(InvoicingAdapterError::StripeError)?;

        for line in invoice.line_items.iter() {
            let create_invoice_line =
                Self::db_invoice_item_to_external(&created_stripe_invoice, invoice, line)?;

            let idempotency_key = format!("{}-{}", invoice.id, line.name);

            self.client
                .create_invoice_item(create_invoice_line, api_key, idempotency_key)
                .await
                .change_context(InvoicingAdapterError::StripeError)?;
        }

        Ok(())
    }
}

impl Stripe {
    pub fn get() -> &'static Self {
        STRIPE.get_or_init(|| Stripe {
            client: Arc::new(stripe_client::client::StripeClient::new()),
        })
    }

    fn external_status_to_service(&self, event_type: String) -> Option<InvoiceExternalStatusEnum> {
        match event_type.as_str() {
            event_type::INVOICE_CREATED => Some(InvoiceExternalStatusEnum::Draft),
            event_type::INVOICE_DELETED => Some(InvoiceExternalStatusEnum::Deleted),
            event_type::INVOICE_PAID => Some(InvoiceExternalStatusEnum::Paid),
            event_type::INVOICE_PAYMENT_FAILED => Some(InvoiceExternalStatusEnum::PaymentFailed),
            event_type::INVOICE_VOIDED => Some(InvoiceExternalStatusEnum::Void),
            event_type::INVOICE_MARKED_UNCOLLECTIBLE => {
                Some(InvoiceExternalStatusEnum::Uncollectible)
            }
            event_type::INVOICE_FINALIZED => Some(InvoiceExternalStatusEnum::Finalized),
            _ => None,
        }
    }

    // for now, this is only about updating the external status
    async fn process_invoice_events(
        &self,
        parsed: Event,
        invoice: Invoice,
        store: Store,
    ) -> Result<bool, errors::AdapterWebhookError> {
        let event_type_clone = parsed.event_type.clone();
        let external_status = match self.external_status_to_service(parsed.event_type) {
            Some(status) => status,
            None => bail!(errors::AdapterWebhookError::EventTypeNotSupported(
                event_type_clone
            )),
        };
        let invoice_id = Uuid::parse_str(invoice.metadata.meteroid_invoice_id.as_str())
            .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

        let tenant_id = invoice.metadata.meteroid_tenant_id;

        store
            .update_invoice_external_status(invoice_id, tenant_id, external_status)
            .await
            .change_context(errors::AdapterWebhookError::DatabaseError)?;

        Ok(true)
    }

    fn db_invoice_to_external<'a>(
        invoice: &'a domain::Invoice,
        stripe_customer: &'a String,
        collection_method: CollectionMethod,
    ) -> CreateInvoice<'a> {
        CreateInvoice {
            auto_advance: Some(false),
            currency: Some(invoice.currency.as_str()),
            collection_method: Some(collection_method),
            days_until_due: match collection_method {
                CollectionMethod::SendInvoice => {
                    if invoice.net_terms > 0 {
                        Some(invoice.net_terms as u32)
                    } else {
                        None
                    }
                }
                _ => None,
            },
            customer: Some(stripe_customer.as_ref()),
            metadata: MeteroidMetadata {
                meteroid_invoice_id: invoice.id.to_string(),
                meteroid_customer_id: invoice.customer_id,
                meteroid_tenant_id: invoice.tenant_id,
            },
        }
    }

    fn db_invoice_item_to_external<'a>(
        stripe_invoice: &'a Invoice,
        invoice: &'a domain::Invoice,
        line: &'a LineItem,
    ) -> Result<CreateInvoiceItem<'a>, InvoicingAdapterError> {
        Ok(CreateInvoiceItem {
            amount: Some(line.total),
            currency: Some(invoice.currency.as_str()),
            customer: stripe_invoice.customer.as_deref().unwrap(),
            description: Some(line.name.as_str()),
            invoice: Some(stripe_invoice.id.as_str()),
            period: Some(Period {
                start: Some(Self::chrono_date_to_timestamp(line.start_date)?),
                end: Some(Self::chrono_date_to_timestamp(line.end_date)?),
            }),
        })
    }

    fn chrono_date_to_timestamp(date: chrono::NaiveDate) -> Result<i64, InvoicingAdapterError> {
        let date_time = date
            .and_hms_opt(0, 0, 0)
            .ok_or(InvoicingAdapterError::InvalidData)?;
        Ok(date_time.and_utc().timestamp())
    }

    fn extract_stripe_collection_method(
        customer: &Customer,
    ) -> Result<CollectionMethod, InvoicingAdapterError> {
        let cm = &Self::extract_stripe_billing_config(customer)?.collection_method;

        match cm {
            StripeCollectionMethod::SendInvoice => Ok(CollectionMethod::SendInvoice),
            StripeCollectionMethod::ChargeAutomatically => {
                Ok(CollectionMethod::ChargeAutomatically)
            }
        }
    }

    fn extract_stripe_customer_id(customer: &Customer) -> Result<String, InvoicingAdapterError> {
        Ok(Self::extract_stripe_billing_config(customer)?
            .customer_id
            .clone())
    }

    fn extract_stripe_billing_config(
        customer: &Customer,
    ) -> Result<&BillingConfigStripe, InvoicingAdapterError> {
        match &customer.billing_config {
            BillingConfig::Stripe(s) => Ok(s),
            BillingConfig::Manual => bail!(InvoicingAdapterError::InvalidData),
        }
    }
}
