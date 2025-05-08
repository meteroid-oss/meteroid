use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use common_domain::ids::string_serde;
use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId};
use error_stack::Report;
use meteroid_store::StoreResult;
use meteroid_store::domain::enums::{
    BillingPeriodEnum, InvoiceStatusEnum, WebhookOutEventTypeEnum,
};
use meteroid_store::domain::outbox_event::{
    CustomerEvent, InvoiceEvent, OutboxEvent, SubscriptionEvent,
};
use meteroid_store::domain::webhooks::{WebhookOutMessageNew, WebhookOutMessagePayload};
use meteroid_store::domain::{Address, ShippingAddress};
use meteroid_store::errors::StoreError;
use o2o::o2o;
use serde::{Serialize, Serializer};

#[derive(Debug, Serialize, o2o)]
#[from_owned(CustomerEvent)]
pub struct Customer {
    #[serde(serialize_with = "string_serde::serialize")]
    #[map(customer_id)]
    pub id: CustomerId,
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
}

#[derive(Debug, Serialize, o2o)]
#[from_owned(SubscriptionEvent)]
pub struct Subscription {
    #[serde(serialize_with = "string_serde::serialize")]
    #[map(subscription_id)]
    pub id: SubscriptionId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    pub customer_alias: Option<String>,
    pub customer_name: String,
    pub billing_day_anchor: u16,
    pub currency: String,
    pub trial_duration: Option<u32>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub billing_start_date: Option<NaiveDate>,
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
    #[serde(serialize_with = "string_serde::serialize")]
    #[map(invoice_id)]
    pub id: InvoiceId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    pub status: InvoiceStatusEnum,
    pub currency: String,
    pub total: i64,      // todo convert to money?
    pub tax_amount: i64, // todo convert to money?
    #[serde(serialize_with = "ser_naive_dt")]
    pub created_at: NaiveDateTime,
}

pub(crate) fn to_webhook_out(evt: OutboxEvent) -> StoreResult<Option<WebhookOutMessageNew>> {
    let event_id = evt.event_id().to_string();
    let out = match evt {
        OutboxEvent::CustomerCreated(event) => {
            let event = Customer::from(*event);
            let payload = serde_json::to_value(event).map_err(|e| {
                Report::from(StoreError::SerdeError(
                    "Failed to serialize payload".to_string(),
                    e,
                ))
            })?;

            Some((
                WebhookOutEventTypeEnum::CustomerCreated,
                WebhookOutMessagePayload::Customer(payload),
            ))
        }
        OutboxEvent::InvoiceCreated(event) => {
            let event = Invoice::from(*event);
            let payload = serde_json::to_value(event).map_err(|e| {
                Report::from(StoreError::SerdeError(
                    "Failed to serialize payload".to_string(),
                    e,
                ))
            })?;

            Some((
                WebhookOutEventTypeEnum::InvoiceCreated,
                WebhookOutMessagePayload::Invoice(payload),
            ))
        }
        OutboxEvent::InvoiceFinalized(event) => {
            let event = Invoice::from(*event);
            let payload = serde_json::to_value(event).map_err(|e| {
                Report::from(StoreError::SerdeError(
                    "Failed to serialize payload".to_string(),
                    e,
                ))
            })?;

            Some((
                WebhookOutEventTypeEnum::InvoiceFinalized,
                WebhookOutMessagePayload::Invoice(payload),
            ))
        }
        OutboxEvent::SubscriptionCreated(event) => {
            let event = Subscription::from(*event);
            let payload = serde_json::to_value(event).map_err(|e| {
                Report::from(StoreError::SerdeError(
                    "Failed to serialize payload".to_string(),
                    e,
                ))
            })?;

            Some((
                WebhookOutEventTypeEnum::SubscriptionCreated,
                WebhookOutMessagePayload::Subscription(payload),
            ))
        }
        OutboxEvent::InvoicePdfGenerated(_) => None,
        OutboxEvent::InvoicePaid(_) => None,
    };

    if let Some((event_type, payload)) = out {
        let webhook = WebhookOutMessageNew {
            id: event_id,
            event_type,
            payload,
        };
        Ok(Some(webhook))
    } else {
        Ok(None)
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
