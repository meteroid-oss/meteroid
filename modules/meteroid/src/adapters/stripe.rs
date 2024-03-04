use error_stack::bail;
use error_stack::Report;
use error_stack::Result;
use hyper::StatusCode;
use secrecy::ExposeSecret;
use secrecy::SecretString;
use stripe_client::invoice::{CollectionMethod, CreateInvoice, MeteroidMetadata};
use stripe_client::invoice::{CreateInvoiceItem, Invoice, Period};
use stripe_client::webhook::Event;
use stripe_client::webhook::EventObject;

use crate::errors;

use super::types::{AdapterCommon, WebhookAdapter};
use crate::adapters::types::{InvoicingAdapter, ParsedRequest};
use crate::datetime::time_utc_now;
use crate::errors::InvoicingAdapterError;
use crate::models::InvoiceLine;
use axum::response::IntoResponse;
use common_domain::StripeSecret;
use cornucopia_async::Params;
use deadpool_postgres::Pool;
use error_stack::ResultExt;
use meteroid_grpc::meteroid::api::customers::v1::customer_billing_config::BillingConfigOneof;
use meteroid_grpc::meteroid::api::customers::v1::{customer_billing_config, Customer};
use meteroid_grpc::meteroid::api::subscriptions::v1::SubscriptionStatus;
use meteroid_repository as db;
use meteroid_repository::{InvoiceExternalStatusEnum, InvoicingProviderEnum};
use stripe_client::webhook::event_type;
use stripe_client::webhook::StripeWebhook;
use uuid::Uuid;

static STRIPE: std::sync::OnceLock<Stripe> = std::sync::OnceLock::new();

#[derive(Debug, Clone)]
pub struct Stripe {
    pub client: stripe_client::client::StripeClient,
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
        db_pool: Pool,
    ) -> Result<bool, errors::AdapterWebhookError> {
        let parsed = StripeWebhook::parse_event(request.json_body.to_string().as_str())
            .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

        let object = parsed.data.object.clone();

        match object {
            EventObject::Invoice(invoice) => {
                self.process_invoice_events(parsed, invoice, db_pool).await
            }
        }?;

        Ok(true)
    }
}

#[async_trait::async_trait]
impl InvoicingAdapter for Stripe {
    async fn send_invoice(
        &self,
        invoice: &meteroid_repository::invoices::Invoice,
        customer: &Customer,
        api_key: SecretString,
    ) -> Result<(), InvoicingAdapterError> {
        match invoice.invoicing_provider {
            InvoicingProviderEnum::STRIPE => {
                let api_key = &StripeSecret(api_key);

                let invoice_lines =
                    serde_json::from_value::<Vec<InvoiceLine>>(invoice.line_items.clone())
                        .change_context(InvoicingAdapterError::InvalidData)?;

                let stripe_customer = Self::extract_stripe_customer_id(customer)?;
                let collection_method = Self::extract_stripe_collection_method(customer)?;

                let create_invoice =
                    Self::db_invoice_to_external(invoice, &stripe_customer, collection_method);

                let created_stripe_invoice = self
                    .client
                    .create_invoice(create_invoice, api_key, invoice.id.to_string())
                    .await
                    .change_context(InvoicingAdapterError::StripeError)?;

                for line in invoice_lines.iter() {
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
    }
}

impl Stripe {
    pub fn get() -> &'static Self {
        STRIPE.get_or_init(|| Stripe {
            client: stripe_client::client::StripeClient::new(),
        })
    }

    fn external_status_to_service(
        &self,
        event_type: String,
    ) -> Option<db::InvoiceExternalStatusEnum> {
        match event_type.as_str() {
            event_type::INVOICE_CREATED => Some(db::InvoiceExternalStatusEnum::DRAFT),
            event_type::INVOICE_DELETED => Some(db::InvoiceExternalStatusEnum::DELETED),
            event_type::INVOICE_PAID => Some(db::InvoiceExternalStatusEnum::PAID),
            event_type::INVOICE_PAYMENT_FAILED => {
                Some(db::InvoiceExternalStatusEnum::PAYMENT_FAILED)
            }
            event_type::INVOICE_VOIDED => Some(db::InvoiceExternalStatusEnum::VOID),
            event_type::INVOICE_MARKED_UNCOLLECTIBLE => {
                Some(db::InvoiceExternalStatusEnum::UNCOLLECTIBLE)
            }
            event_type::INVOICE_FINALIZED => Some(db::InvoiceExternalStatusEnum::FINALIZED),
            _ => None,
        }
    }

    // for now, this is only about updating the external status
    async fn process_invoice_events(
        &self,
        parsed: Event,
        invoice: Invoice,
        db_pool: Pool,
    ) -> Result<bool, errors::AdapterWebhookError> {
        let mut conn = db_pool
            .get()
            .await
            .change_context(errors::AdapterWebhookError::DatabaseError)?;

        let transaction = conn
            .transaction()
            .await
            .change_context(errors::AdapterWebhookError::DatabaseError)?;

        let event_type_clone = parsed.event_type.clone();
        let external_status = match self.external_status_to_service(parsed.event_type) {
            Some(status) => status,
            None => bail!(errors::AdapterWebhookError::EventTypeNotSupported(
                event_type_clone
            )),
        };
        let invoice_id = Uuid::parse_str(invoice.metadata.meteroid_invoice_id.as_str())
            .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

        db::invoices::update_invoice_external_status()
            .params(
                &transaction,
                &db::invoices::UpdateInvoiceExternalStatusParams {
                    id: invoice_id,
                    external_status,
                },
            )
            .one()
            .await
            .change_context(errors::AdapterWebhookError::DatabaseError)?;

        let invoice = db::invoices::invoice_by_id()
            .bind(&transaction, &invoice_id)
            .one()
            .await
            .change_context(errors::AdapterWebhookError::DatabaseError)?;

        if let Some(_) = Self::invoice_status_to_subscription_status(external_status) {
            db::subscriptions::activate_subscription()
                .params(
                    &transaction,
                    &db::subscriptions::ActivateSubscriptionParams {
                        id: invoice.subscription_id,
                        activated_at: time_utc_now(),
                    },
                )
                .await
                .change_context(errors::AdapterWebhookError::DatabaseError)?;
        }

        transaction
            .commit()
            .await
            .change_context(errors::AdapterWebhookError::DatabaseError)?;

        Ok(true)
    }

    fn invoice_status_to_subscription_status(
        invoice_status: InvoiceExternalStatusEnum,
    ) -> Option<SubscriptionStatus> {
        match invoice_status {
            InvoiceExternalStatusEnum::PAID => Some(SubscriptionStatus::Active),
            // todo what if payment failed? should we leave subscription Pending?
            _ => None,
        }
    }

    fn db_invoice_to_external<'a>(
        invoice: &'a meteroid_repository::invoices::Invoice,
        stripe_customer: &'a String,
        collection_method: CollectionMethod,
    ) -> CreateInvoice<'a> {
        CreateInvoice {
            auto_advance: Some(false),
            currency: Some(invoice.currency.as_str()),
            collection_method: Some(collection_method),
            days_until_due: match collection_method {
                CollectionMethod::SendInvoice => invoice.days_until_due.map(|d| d as u32),
                _ => None,
            },
            customer: Some(stripe_customer.as_ref()),
            metadata: MeteroidMetadata {
                meteroid_invoice_id: invoice.id.to_string(),
                meteroid_customer_id: invoice.customer_id.to_string(),
                meteroid_tenant_id: invoice.tenant_id.to_string(),
            },
        }
    }

    fn db_invoice_item_to_external<'a>(
        stripe_invoice: &'a Invoice,
        invoice: &'a meteroid_repository::invoices::Invoice,
        line: &'a InvoiceLine,
    ) -> Result<CreateInvoiceItem<'a>, InvoicingAdapterError> {
        Ok(CreateInvoiceItem {
            amount: Some(line.total),
            currency: Some(invoice.currency.as_str()),
            customer: stripe_invoice.customer.as_deref().unwrap(),
            description: Some(line.name.as_str()),
            invoice: Some(stripe_invoice.id.as_str()),
            period: line
                .period
                .as_ref()
                .map(|period| {
                    Ok::<Period, Report<InvoicingAdapterError>>(Period {
                        start: Some(Self::chrono_date_to_timestamp(period.from)?),
                        end: Some(Self::chrono_date_to_timestamp(period.to)?),
                    })
                })
                .transpose()?,
        })
    }

    fn chrono_date_to_timestamp(date: chrono::NaiveDate) -> Result<i64, InvoicingAdapterError> {
        let date_time = date
            .and_hms_opt(0, 0, 0)
            .ok_or(InvoicingAdapterError::InvalidData)?;
        Ok(date_time.timestamp())
    }

    fn extract_stripe_collection_method(
        customer: &Customer,
    ) -> Result<CollectionMethod, InvoicingAdapterError> {
        let cm: customer_billing_config::stripe::CollectionMethod =
            Self::extract_stripe_billing_config(customer)?
                .collection_method
                .try_into()
                .map_err(|_| Report::new(InvoicingAdapterError::InvalidData))?;

        match cm {
            customer_billing_config::stripe::CollectionMethod::SendInvoice => {
                Ok(CollectionMethod::SendInvoice)
            }
            customer_billing_config::stripe::CollectionMethod::ChargeAutomatically => {
                Ok(CollectionMethod::ChargeAutomatically)
            }
        }
    }

    fn extract_stripe_customer_id(customer: &Customer) -> Result<String, InvoicingAdapterError> {
        Ok(Self::extract_stripe_billing_config(customer)?.customer_id)
    }

    fn extract_stripe_billing_config(
        customer: &Customer,
    ) -> Result<customer_billing_config::Stripe, InvoicingAdapterError> {
        customer
            .billing_config
            .clone()
            .and_then(|bc| bc.billing_config_oneof)
            .map(|oneof| match oneof {
                BillingConfigOneof::Stripe(stripe) => stripe,
            })
            .ok_or(Report::new(InvoicingAdapterError::InvalidData))
    }
}
