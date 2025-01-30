use crate::workers::kafka::outbox::{parse_outbox_event, EventType, OutboxEvent};
use crate::workers::kafka::processor::MessageHandler;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use error_stack::Report;
use meteroid_store::domain::enums::{
    BillingPeriodEnum, InvoiceStatusEnum, WebhookOutEventTypeEnum,
};
use meteroid_store::domain::outbox_event::{CustomerEvent, InvoiceEvent, SubscriptionEvent};
use meteroid_store::domain::webhooks::{
    WebhookOutCreateMessageResult, WebhookOutMessageNew, WebhookOutMessagePayload,
};
use meteroid_store::domain::{Address, ShippingAddress};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::webhooks::WebhooksInterface;
use meteroid_store::Store;
use o2o::o2o;
use serde::{Serialize, Serializer};
use std::sync::Arc;

pub struct WebhookHandler {
    store: Arc<Store>,
}

impl WebhookHandler {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl MessageHandler for WebhookHandler {
    async fn handle(
        &self,
        message: &rdkafka::message::BorrowedMessage<'_>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(event) = parse_outbox_event(message) {
            log::info!("Processing message: {:?}", event);

            let tenant_id = event.tenant_id;

            let wh: Option<WebhookOutMessageNew> = event.try_into()?;

            if let Some(wh) = wh {
                let webhook_type = wh.event_type.to_string();
                let event_id = wh.id.clone();

                let result = self.store.insert_webhook_message_out(tenant_id, wh).await?;

                match result {
                    WebhookOutCreateMessageResult::Created(_) => {
                        log::info!("Sent {} webhook with id {}", webhook_type, event_id)
                    }
                    WebhookOutCreateMessageResult::Conflict => {
                        log::warn!(
                            "Skipped {} webhook with id {} as it already exists",
                            webhook_type,
                            event_id
                        )
                    }
                    WebhookOutCreateMessageResult::NotFound => {
                        log::warn!(
                            "Skipped {} webhook with id {} as the webhooks seem to not be configured for tenant {}",
                            webhook_type,
                            event_id,
                            tenant_id
                        )
                    }
                    WebhookOutCreateMessageResult::SvixNotConfigured => {
                        log::warn!(
                            "Skipped {} webhook with id {} as svix client not configured",
                            webhook_type,
                            event_id
                        )
                    }
                }
            } else {
                log::debug!("Skipping outbox message");
            }
        } else {
            log::debug!("Skipping message");
        }

        Ok(())
    }
}

#[derive(Debug, Serialize, o2o)]
#[from_owned(CustomerEvent)]
pub struct Customer {
    #[map(local_id)]
    pub id: String,
    pub name: String,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
}

#[derive(Debug, Serialize, o2o)]
#[from_owned(SubscriptionEvent)]
pub struct Subscription {
    #[map(local_id)]
    pub id: String,
    #[map(customer_local_id)]
    pub customer_id: String,
    pub customer_alias: Option<String>,
    pub customer_name: String,
    pub billing_day: i16,
    pub currency: String,
    pub trial_start_date: Option<NaiveDate>,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub plan_name: String,
    pub version: u32,
    #[serde(serialize_with = "ser_naive_dt")]
    pub created_at: NaiveDateTime,
    pub net_terms: u32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    #[serde(serialize_with = "ser_naive_dt_opt")]
    pub activated_at: Option<NaiveDateTime>,
    #[serde(serialize_with = "ser_naive_dt_opt")]
    pub canceled_at: Option<NaiveDateTime>,
    pub cancellation_reason: Option<String>,
    pub mrr_cents: u64,
    pub period: BillingPeriodEnum,
}

#[derive(Debug, Serialize, o2o)]
#[from_owned(InvoiceEvent)]
pub struct Invoice {
    #[map(local_id)]
    pub id: String,
    #[map(customer_local_id)]
    pub customer_id: String,
    pub status: InvoiceStatusEnum,
    pub currency: String,
    pub total: i64,      // todo convert to money?
    pub tax_amount: i64, // todo convert to money?
    #[serde(serialize_with = "ser_naive_dt")]
    pub created_at: NaiveDateTime,
}

impl TryInto<Option<WebhookOutMessageNew>> for OutboxEvent {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<Option<WebhookOutMessageNew>, Self::Error> {
        let (event_type, payload) = match self.event_type {
            EventType::CustomerCreated(event) => {
                let event = Customer::from(*event);
                let payload = serde_json::to_value(event).map_err(|e| {
                    Report::from(StoreError::SerdeError(
                        "Failed to serialize payload".to_string(),
                        e,
                    ))
                })?;

                (
                    WebhookOutEventTypeEnum::CustomerCreated,
                    WebhookOutMessagePayload::Customer(payload),
                )
            }
            EventType::SubscriptionCreated(event) => {
                let event = Subscription::from(*event);
                let payload = serde_json::to_value(event).map_err(|e| {
                    Report::from(StoreError::SerdeError(
                        "Failed to serialize payload".to_string(),
                        e,
                    ))
                })?;

                (
                    WebhookOutEventTypeEnum::SubscriptionCreated,
                    WebhookOutMessagePayload::Subscription(payload),
                )
            }
            EventType::InvoiceCreated(event) => {
                let event = Invoice::from(*event);
                let payload = serde_json::to_value(event).map_err(|e| {
                    Report::from(StoreError::SerdeError(
                        "Failed to serialize payload".to_string(),
                        e,
                    ))
                })?;

                (
                    WebhookOutEventTypeEnum::InvoiceCreated,
                    WebhookOutMessagePayload::Invoice(payload),
                )
            }
            EventType::InvoiceFinalized(event) => {
                let event = Invoice::from(*event);
                let payload = serde_json::to_value(event).map_err(|e| {
                    Report::from(StoreError::SerdeError(
                        "Failed to serialize payload".to_string(),
                        e,
                    ))
                })?;

                (
                    WebhookOutEventTypeEnum::InvoiceFinalized,
                    WebhookOutMessagePayload::Invoice(payload),
                )
            }
            _ => return Ok(None),
        };

        let webhook = WebhookOutMessageNew {
            id: self.id,
            event_type,
            payload,
        };

        Ok(Some(webhook))
    }
}

fn ser_naive_dt<S>(datetime: &NaiveDateTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let formatted = format_utc(&datetime.and_utc());
    serializer.serialize_str(&formatted)
}

fn ser_naive_dt_opt<S>(datetime: &Option<NaiveDateTime>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match datetime {
        Some(datetime) => {
            let formatted = format_utc(&datetime.and_utc());
            serializer.serialize_str(&formatted)
        }
        None => serializer.serialize_none(),
    }
}

fn format_utc(datetime: &DateTime<Utc>) -> String {
    datetime.to_rfc3339_opts(SecondsFormat::Millis, true)
}
