use crate::api_rest::invoices::model::InvoiceStatus;
use crate::api_rest::metrics::model::{
    BillingMetricAggregateEnum, MetricSegmentationMatrix, UnitConversionRoundingEnum,
};
use crate::api_rest::model::BillingPeriodEnum;
use crate::api_rest::subscriptions::model::SubscriptionStatusEnum;
use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use common_domain::ids::{
    BillableMetricId, CreditNoteId, CustomerId, EventId, InvoiceId, ProductFamilyId, ProductId,
    QuoteId, SubscriptionId, string_serde, string_serde_opt,
};
use meteroid_store::domain::outbox_event::{
    BillableMetricEvent, CreditNoteEvent, CustomerEvent, InvoiceEvent, QuoteAcceptedEvent,
    QuoteConvertedEvent, SubscriptionEvent,
};
use o2o::o2o;
use serde::{Serialize, Serializer};
use serde_with::skip_serializing_none;
use strum::{Display, EnumIter, EnumString};
use svix::api::MessageIn;

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[schema(as = CustomerEventData)]
#[from_owned(CustomerEvent)]
pub struct WebhookOutCustomerEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: String,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[schema(as = InvoiceEventData)]
#[from_owned(InvoiceEvent)]
pub struct WebhookOutInvoiceEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub invoice_id: InvoiceId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    #[from(~.into())]
    pub status: InvoiceStatus,
    pub currency: String,
    pub total: i64,
    pub tax_amount: i64,
    #[serde(serialize_with = "ser_naive_dt")]
    pub created_at: NaiveDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[schema(as = SubscriptionEventData)]
#[from_owned(SubscriptionEvent)]
pub struct WebhookOutSubscriptionEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub subscription_id: SubscriptionId,
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
    pub mrr_cents: u64,
    #[from(~.into())]
    pub period: BillingPeriodEnum,
    #[from(~.into())]
    pub status: SubscriptionStatusEnum,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[schema(as = MetricEventData)]
#[from_owned(BillableMetricEvent)]
pub struct WebhookOutMetricEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub metric_id: BillableMetricId,
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    #[from(~.into())]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    #[from(~.map(Into::into))]
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    #[from(~.map(Into::into))]
    pub segmentation_matrix: Option<MetricSegmentationMatrix>,
    pub usage_group_key: Option<String>,
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "string_serde::serialize")]
    pub product_family_id: ProductFamilyId,
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub product_id: Option<ProductId>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = QuoteEventData)]
pub struct WebhookOutQuoteEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub quote_id: QuoteId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub subscription_id: Option<SubscriptionId>,
}

impl From<QuoteAcceptedEvent> for WebhookOutQuoteEventData {
    fn from(value: QuoteAcceptedEvent) -> Self {
        WebhookOutQuoteEventData {
            quote_id: value.quote_id,
            customer_id: value.customer_id,
            subscription_id: None,
        }
    }
}

impl From<QuoteConvertedEvent> for WebhookOutQuoteEventData {
    fn from(value: QuoteConvertedEvent) -> Self {
        WebhookOutQuoteEventData {
            quote_id: value.quote_id,
            customer_id: value.customer_id,
            subscription_id: Some(value.subscription_id),
        }
    }
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditNoteStatus {
    Draft,
    Finalized,
    Voided,
}

impl From<meteroid_store::domain::enums::CreditNoteStatus> for CreditNoteStatus {
    fn from(value: meteroid_store::domain::enums::CreditNoteStatus) -> Self {
        match value {
            meteroid_store::domain::enums::CreditNoteStatus::Draft => CreditNoteStatus::Draft,
            meteroid_store::domain::enums::CreditNoteStatus::Finalized => {
                CreditNoteStatus::Finalized
            }
            meteroid_store::domain::enums::CreditNoteStatus::Voided => CreditNoteStatus::Voided,
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[schema(as = CreditNoteEventData)]
#[from_owned(CreditNoteEvent)]
pub struct WebhookOutCreditNoteEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub credit_note_id: CreditNoteId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub invoice_id: InvoiceId,
    #[from(~.into())]
    pub status: CreditNoteStatus,
    pub currency: String,
    pub total: i64,
    pub tax_amount: i64,
    pub refunded_amount_cents: i64,
    pub credited_amount_cents: i64,
    #[serde(serialize_with = "ser_naive_dt")]
    pub created_at: NaiveDateTime,
}

/// Event-specific webhook schemas for type-safe webhook payloads

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = CustomerEvent)]
pub struct WebhookOutCustomerEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutCustomerEventData,
    #[serde(serialize_with = "ser_naive_dt")]
    pub timestamp: NaiveDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = InvoiceEvent)]
pub struct WebhookOutInvoiceEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutInvoiceEventData,
    #[serde(serialize_with = "ser_naive_dt")]
    pub timestamp: NaiveDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = SubscriptionEvent)]
pub struct WebhookOutSubscriptionEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutSubscriptionEventData,
    #[serde(serialize_with = "ser_naive_dt")]
    pub timestamp: NaiveDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = MetricEvent)]
pub struct WebhookOutMetricEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutMetricEventData,
    #[serde(serialize_with = "ser_naive_dt")]
    pub timestamp: NaiveDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = QuoteEvent)]
pub struct WebhookOutQuoteEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutQuoteEventData,
    #[serde(serialize_with = "ser_naive_dt")]
    pub timestamp: NaiveDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = CreditNoteEvent)]
pub struct WebhookOutCreditNoteEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutCreditNoteEventData,
    #[serde(serialize_with = "ser_naive_dt")]
    pub timestamp: NaiveDateTime,
}

impl TryInto<MessageIn> for WebhookOutCustomerEvent {
    type Error = serde_json::Error;

    fn try_into(self) -> Result<MessageIn, Self::Error> {
        let result = serde_json::to_value(&self)?;
        Ok(MessageIn::new(self.event_type.to_string(), result))
    }
}

impl TryInto<MessageIn> for WebhookOutInvoiceEvent {
    type Error = serde_json::Error;

    fn try_into(self) -> Result<MessageIn, Self::Error> {
        let result = serde_json::to_value(&self)?;
        Ok(MessageIn::new(self.event_type.to_string(), result))
    }
}

impl TryInto<MessageIn> for WebhookOutSubscriptionEvent {
    type Error = serde_json::Error;

    fn try_into(self) -> Result<MessageIn, Self::Error> {
        let result = serde_json::to_value(&self)?;
        Ok(MessageIn::new(self.event_type.to_string(), result))
    }
}

impl TryInto<MessageIn> for WebhookOutMetricEvent {
    type Error = serde_json::Error;

    fn try_into(self) -> Result<MessageIn, Self::Error> {
        let result = serde_json::to_value(&self)?;
        Ok(MessageIn::new(self.event_type.to_string(), result))
    }
}

impl TryInto<MessageIn> for WebhookOutQuoteEvent {
    type Error = serde_json::Error;

    fn try_into(self) -> Result<MessageIn, Self::Error> {
        let result = serde_json::to_value(&self)?;
        Ok(MessageIn::new(self.event_type.to_string(), result))
    }
}

impl TryInto<MessageIn> for WebhookOutCreditNoteEvent {
    type Error = serde_json::Error;

    fn try_into(self) -> Result<MessageIn, Self::Error> {
        let result = serde_json::to_value(&self)?;
        Ok(MessageIn::new(self.event_type.to_string(), result))
    }
}

#[derive(
    Debug,
    Copy,
    Clone,
    Eq,
    PartialEq,
    Hash,
    Display,
    EnumIter,
    EnumString,
    Serialize,
    utoipa::ToSchema,
)]
#[schema(as = EventType)]
pub enum WebhookOutEventTypeEnum {
    #[strum(serialize = "metric.created")]
    #[serde(rename = "metric.created")]
    BillableMetricCreated,
    #[strum(serialize = "customer.created")]
    #[serde(rename = "customer.created")]
    CustomerCreated,
    #[strum(serialize = "subscription.created")]
    #[serde(rename = "subscription.created")]
    SubscriptionCreated,
    #[strum(serialize = "invoice.created")]
    #[serde(rename = "invoice.created")]
    InvoiceCreated,
    #[strum(serialize = "invoice.finalized")]
    #[serde(rename = "invoice.finalized")]
    InvoiceFinalized,
    #[strum(serialize = "invoice.paid")]
    #[serde(rename = "invoice.paid")]
    InvoicePaid,
    #[strum(serialize = "invoice.voided")]
    #[serde(rename = "invoice.voided")]
    InvoiceVoided,
    #[strum(serialize = "quote.accepted")]
    #[serde(rename = "quote.accepted")]
    QuoteAccepted,
    #[strum(serialize = "quote.converted")]
    #[serde(rename = "quote.converted")]
    QuoteConverted,
    #[strum(serialize = "credit_note.created")]
    #[serde(rename = "credit_note.created")]
    CreditNoteCreated,
    #[strum(serialize = "credit_note.finalized")]
    #[serde(rename = "credit_note.finalized")]
    CreditNoteFinalized,
    #[strum(serialize = "credit_note.voided")]
    #[serde(rename = "credit_note.voided")]
    CreditNoteVoided,
}

#[derive(Debug, Display, EnumIter, EnumString, Copy, Clone)]
pub enum WebhookOutEventGroupEnum {
    #[strum(serialize = "customer")]
    Customer,
    #[strum(serialize = "subscription")]
    Subscription,
    #[strum(serialize = "invoice")]
    Invoice,
    #[strum(serialize = "metric")]
    BillableMetric,
    #[strum(serialize = "quote")]
    Quote,
    #[strum(serialize = "credit_note")]
    CreditNote,
}

impl WebhookOutEventGroupEnum {
    pub fn schema_name(&self) -> &'static str {
        match self {
            WebhookOutEventGroupEnum::Customer => "CustomerEvent",
            WebhookOutEventGroupEnum::Subscription => "SubscriptionEvent",
            WebhookOutEventGroupEnum::Invoice => "InvoiceEvent",
            WebhookOutEventGroupEnum::BillableMetric => "MetricEvent",
            WebhookOutEventGroupEnum::Quote => "QuoteEvent",
            WebhookOutEventGroupEnum::CreditNote => "CreditNoteEvent",
        }
    }
}

impl WebhookOutEventTypeEnum {
    pub fn group(&self) -> WebhookOutEventGroupEnum {
        match self {
            WebhookOutEventTypeEnum::CustomerCreated => WebhookOutEventGroupEnum::Customer,
            WebhookOutEventTypeEnum::SubscriptionCreated => WebhookOutEventGroupEnum::Subscription,
            WebhookOutEventTypeEnum::InvoiceCreated => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoiceFinalized => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoicePaid => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoiceVoided => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::BillableMetricCreated => {
                WebhookOutEventGroupEnum::BillableMetric
            }
            WebhookOutEventTypeEnum::QuoteAccepted => WebhookOutEventGroupEnum::Quote,
            WebhookOutEventTypeEnum::QuoteConverted => WebhookOutEventGroupEnum::Quote,
            WebhookOutEventTypeEnum::CreditNoteCreated => WebhookOutEventGroupEnum::CreditNote,
            WebhookOutEventTypeEnum::CreditNoteFinalized => WebhookOutEventGroupEnum::CreditNote,
            WebhookOutEventTypeEnum::CreditNoteVoided => WebhookOutEventGroupEnum::CreditNote,
        }
    }

    pub fn description(&self) -> String {
        match self {
            WebhookOutEventTypeEnum::CustomerCreated => "A new customer was created".to_string(),
            WebhookOutEventTypeEnum::SubscriptionCreated => {
                "A new subscription was created".to_string()
            }
            WebhookOutEventTypeEnum::InvoiceCreated => "A new invoice was created".to_string(),
            WebhookOutEventTypeEnum::InvoiceFinalized => "An invoice was finalized".to_string(),
            WebhookOutEventTypeEnum::InvoicePaid => "An invoice was paid".to_string(),
            WebhookOutEventTypeEnum::InvoiceVoided => "An invoice was voided".to_string(),
            WebhookOutEventTypeEnum::BillableMetricCreated => {
                "A new billable metric was created".to_string()
            }
            WebhookOutEventTypeEnum::QuoteAccepted => "A quote was accepted".to_string(),
            WebhookOutEventTypeEnum::QuoteConverted => {
                "A quote was converted into subscription".to_string()
            }
            WebhookOutEventTypeEnum::CreditNoteCreated => {
                "A new credit note was created".to_string()
            }
            WebhookOutEventTypeEnum::CreditNoteFinalized => {
                "A credit note was finalized".to_string()
            }
            WebhookOutEventTypeEnum::CreditNoteVoided => "A credit note was voided".to_string(),
        }
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
