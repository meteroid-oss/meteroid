use crate::domain::connectors::ConnectionMeta;
use crate::domain::enums::{BillingPeriodEnum, InvoiceStatusEnum};
use crate::domain::pgmq::{PgmqMessage, PgmqMessageNew};
use crate::domain::enums::CreditNoteStatus;
use crate::domain::{
    Address, BillableMetric, BillingMetricAggregateEnum, CreditNote, Customer, Invoice,
    PaymentStatusEnum, PaymentTransaction, PaymentTypeEnum, Quote, SegmentationMatrix, ShippingAddress,
    Subscription, SubscriptionStatusEnum, UnitConversionRoundingEnum,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::{StoreResult, json_value_serde};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    BankAccountId, BaseId, BillableMetricId, ConnectorId, CreditNoteId, CustomerId,
    CustomerPaymentMethodId,EventId, InvoiceId, PaymentTransactionId, PlanId, PlanVersionId, ProductFamilyId, ProductId,
    QuoteId, StoredDocumentId, SubscriptionId, TenantId,
};
use diesel_models::outbox_event::OutboxEventRowNew;
use diesel_models::pgmq::PgmqMessageRowNew;
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use strum::Display;
use uuid::Uuid;

#[derive(Display, Debug, Serialize, Deserialize)]
pub enum OutboxEvent {
    CustomerCreated(Box<CustomerEvent>),
    CustomerUpdated(Box<CustomerEvent>),
    BillableMetricCreated(Box<BillableMetricEvent>),
    InvoiceCreated(Box<InvoiceEvent>),
    InvoiceFinalized(Box<InvoiceEvent>),
    InvoicePaid(Box<InvoiceEvent>),
    InvoiceVoided(Box<InvoiceEvent>),
    // only triggered at finalization. Other pdfs (other lang etc) do not trigger this.
    InvoiceAccountingPdfGenerated(Box<InvoicePdfGeneratedEvent>),
    CreditNoteCreated(Box<CreditNoteEvent>),
    CreditNoteFinalized(Box<CreditNoteEvent>),
    CreditNoteVoided(Box<CreditNoteEvent>),
    SubscriptionCreated(Box<SubscriptionEvent>),
    PaymentTransactionSaved(Box<PaymentTransactionEvent>),
    QuoteAccepted(Box<QuoteAcceptedEvent>),
    QuoteConverted(Box<QuoteConvertedEvent>),
}

#[derive(Display, Debug, Serialize, Deserialize, PartialEq)]
pub enum EventType {
    CustomerCreated,
    CustomerUpdated,
    BillableMetricCreated,
    InvoiceCreated,
    InvoiceFinalized,
    InvoicePaid,
    InvoiceVoided,
    InvoiceAccountingPdfGenerated,
    CreditNoteCreated,
    CreditNoteFinalized,
    CreditNoteVoided,
    SubscriptionCreated,
    PaymentTransactionReceived,
    QuoteAccepted,
    QuoteConverted,
}

json_value_serde!(OutboxEvent);

impl OutboxEvent {
    pub fn event_id(&self) -> EventId {
        match self {
            OutboxEvent::CustomerCreated(event) => event.id,
            OutboxEvent::CustomerUpdated(event) => event.id,
            OutboxEvent::BillableMetricCreated(event) => event.id,
            OutboxEvent::InvoiceCreated(event) => event.id,
            OutboxEvent::InvoiceFinalized(event) => event.id,
            OutboxEvent::InvoicePaid(event) => event.id,
            OutboxEvent::InvoiceVoided(event) => event.id,
            OutboxEvent::InvoiceAccountingPdfGenerated(event) => event.id,
            OutboxEvent::CreditNoteCreated(event) => event.id,
            OutboxEvent::CreditNoteFinalized(event) => event.id,
            OutboxEvent::CreditNoteVoided(event) => event.id,
            OutboxEvent::SubscriptionCreated(event) => event.id,
            OutboxEvent::PaymentTransactionSaved(event) => event.id,
            OutboxEvent::QuoteAccepted(event) => event.id,
            OutboxEvent::QuoteConverted(event) => event.id,
        }
    }

    pub fn tenant_id(&self) -> TenantId {
        match self {
            OutboxEvent::CustomerCreated(event) => event.tenant_id,
            OutboxEvent::CustomerUpdated(event) => event.tenant_id,
            OutboxEvent::BillableMetricCreated(event) => event.tenant_id,
            OutboxEvent::InvoiceCreated(event) => event.tenant_id,
            OutboxEvent::InvoiceFinalized(event) => event.tenant_id,
            OutboxEvent::InvoicePaid(event) => event.tenant_id,
            OutboxEvent::InvoiceVoided(event) => event.tenant_id,
            OutboxEvent::InvoiceAccountingPdfGenerated(event) => event.tenant_id,
            OutboxEvent::CreditNoteCreated(event) => event.tenant_id,
            OutboxEvent::CreditNoteFinalized(event) => event.tenant_id,
            OutboxEvent::CreditNoteVoided(event) => event.tenant_id,
            OutboxEvent::SubscriptionCreated(event) => event.tenant_id,
            OutboxEvent::PaymentTransactionSaved(event) => event.tenant_id,
            OutboxEvent::QuoteAccepted(event) => event.tenant_id,
            OutboxEvent::QuoteConverted(event) => event.tenant_id,
        }
    }

    pub fn aggregate_id(&self) -> Uuid {
        match self {
            OutboxEvent::CustomerCreated(event) => event.customer_id.as_uuid(),
            OutboxEvent::CustomerUpdated(event) => event.customer_id.as_uuid(),
            OutboxEvent::BillableMetricCreated(event) => event.metric_id.as_uuid(),
            OutboxEvent::InvoiceCreated(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoiceFinalized(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoicePaid(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoiceVoided(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoiceAccountingPdfGenerated(event) => event.invoice_id.as_uuid(),
            OutboxEvent::CreditNoteCreated(event) => event.credit_note_id.as_uuid(),
            OutboxEvent::CreditNoteFinalized(event) => event.credit_note_id.as_uuid(),
            OutboxEvent::CreditNoteVoided(event) => event.credit_note_id.as_uuid(),
            OutboxEvent::SubscriptionCreated(event) => event.subscription_id.as_uuid(),
            OutboxEvent::PaymentTransactionSaved(event) => event.payment_transaction_id.as_uuid(),
            OutboxEvent::QuoteAccepted(event) => event.quote_id.as_uuid(),
            OutboxEvent::QuoteConverted(event) => event.quote_id.as_uuid(),
        }
    }

    pub fn aggregate_type(&self) -> String {
        match self {
            OutboxEvent::CustomerCreated(_) => "Customer".to_string(),
            OutboxEvent::CustomerUpdated(_) => "Customer".to_string(),
            OutboxEvent::BillableMetricCreated(_) => "BillableMetric".to_string(),
            OutboxEvent::InvoiceCreated(_) => "Invoice".to_string(),
            OutboxEvent::InvoiceFinalized(_) => "Invoice".to_string(),
            OutboxEvent::InvoicePaid(_) => "Invoice".to_string(),
            OutboxEvent::InvoiceVoided(_) => "Invoice".to_string(),
            OutboxEvent::InvoiceAccountingPdfGenerated(_) => "Invoice".to_string(),
            OutboxEvent::CreditNoteCreated(_) => "CreditNote".to_string(),
            OutboxEvent::CreditNoteFinalized(_) => "CreditNote".to_string(),
            OutboxEvent::CreditNoteVoided(_) => "CreditNote".to_string(),
            OutboxEvent::SubscriptionCreated(_) => "Subscription".to_string(),
            OutboxEvent::PaymentTransactionSaved(_) => "PaymentTransaction".to_string(),
            OutboxEvent::QuoteAccepted(_) => "Quote".to_string(),
            OutboxEvent::QuoteConverted(_) => "Quote".to_string(),
        }
    }

    pub fn event_type(&self) -> EventType {
        match self {
            OutboxEvent::CustomerCreated(_) => EventType::CustomerCreated,
            OutboxEvent::CustomerUpdated(_) => EventType::CustomerUpdated,
            OutboxEvent::BillableMetricCreated(_) => EventType::BillableMetricCreated,
            OutboxEvent::InvoiceCreated(_) => EventType::InvoiceCreated,
            OutboxEvent::InvoiceFinalized(_) => EventType::InvoiceFinalized,
            OutboxEvent::InvoicePaid(_) => EventType::InvoicePaid,
            OutboxEvent::InvoiceVoided(_) => EventType::InvoiceVoided,
            OutboxEvent::InvoiceAccountingPdfGenerated(_) => {
                EventType::InvoiceAccountingPdfGenerated
            }
            OutboxEvent::CreditNoteCreated(_) => EventType::CreditNoteCreated,
            OutboxEvent::CreditNoteFinalized(_) => EventType::CreditNoteFinalized,
            OutboxEvent::CreditNoteVoided(_) => EventType::CreditNoteVoided,
            OutboxEvent::SubscriptionCreated(_) => EventType::SubscriptionCreated,
            OutboxEvent::PaymentTransactionSaved(_) => EventType::PaymentTransactionReceived,
            OutboxEvent::QuoteAccepted(_) => EventType::QuoteAccepted,
            OutboxEvent::QuoteConverted(_) => EventType::QuoteConverted,
        }
    }

    pub fn customer_created(event: CustomerEvent) -> OutboxEvent {
        OutboxEvent::CustomerCreated(Box::new(event))
    }

    pub fn customer_updated(event: CustomerEvent) -> OutboxEvent {
        OutboxEvent::CustomerUpdated(Box::new(event))
    }

    pub fn billable_metric_created(event: BillableMetricEvent) -> OutboxEvent {
        OutboxEvent::BillableMetricCreated(Box::new(event))
    }

    pub fn invoice_created(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoiceCreated(Box::new(event))
    }

    pub fn invoice_finalized(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoiceFinalized(Box::new(event))
    }

    pub fn invoice_paid(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoicePaid(Box::new(event))
    }

    pub fn invoice_voided(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoiceVoided(Box::new(event))
    }

    pub fn invoice_pdf_generated(event: InvoicePdfGeneratedEvent) -> OutboxEvent {
        OutboxEvent::InvoiceAccountingPdfGenerated(Box::new(event))
    }

    pub fn credit_note_created(event: CreditNoteEvent) -> OutboxEvent {
        OutboxEvent::CreditNoteCreated(Box::new(event))
    }

    pub fn credit_note_finalized(event: CreditNoteEvent) -> OutboxEvent {
        OutboxEvent::CreditNoteFinalized(Box::new(event))
    }

    pub fn credit_note_voided(event: CreditNoteEvent) -> OutboxEvent {
        OutboxEvent::CreditNoteVoided(Box::new(event))
    }

    pub fn subscription_created(event: SubscriptionEvent) -> OutboxEvent {
        OutboxEvent::SubscriptionCreated(Box::new(event))
    }

    pub fn payment_transaction_saved(event: PaymentTransactionEvent) -> OutboxEvent {
        OutboxEvent::PaymentTransactionSaved(Box::new(event))
    }

    pub fn quote_accepted(event: QuoteAcceptedEvent) -> OutboxEvent {
        OutboxEvent::QuoteAccepted(Box::new(event))
    }

    pub fn quote_converted(event: QuoteConvertedEvent) -> OutboxEvent {
        OutboxEvent::QuoteConverted(Box::new(event))
    }

    fn payload_json(&self) -> StoreResult<serde_json::Value> {
        serde_json::to_value(self).map_err(|e| {
            Report::from(StoreError::SerdeError(
                "Failed to serialize payload".to_string(),
                e,
            ))
        })
    }
}

impl TryInto<OutboxEventRowNew> for OutboxEvent {
    type Error = StoreErrorReport;
    fn try_into(self) -> Result<OutboxEventRowNew, Self::Error> {
        Ok(OutboxEventRowNew {
            id: self.event_id(),
            tenant_id: self.tenant_id(),
            aggregate_id: self.aggregate_id().to_string(),
            aggregate_type: self.aggregate_type(),
            event_type: self.event_type().to_string(),
            payload: self.payload_json()?,
        })
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_owned(Customer)]
pub struct CustomerEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[map(id)]
    pub customer_id: CustomerId,
    pub tenant_id: TenantId,
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub vat_number: Option<String>,
    pub bank_account_id: Option<BankAccountId>,
    pub conn_meta: Option<ConnectionMeta>,
}

impl CustomerEvent {
    pub fn get_pennylane_id(&self, connector_id: ConnectorId) -> Option<i64> {
        self.conn_meta
            .as_ref()
            .and_then(|meta| meta.get_pennylane_id(connector_id))
    }
}

// TODO golden tests
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[map_owned(BillableMetric)]
#[ghosts(archived_at: None, updated_at: None, synced_at: None, sync_error: None)]
pub struct BillableMetricEvent {
    #[ghost(EventId::new())]
    pub id: EventId,
    #[map(id)]
    pub metric_id: BillableMetricId,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    pub segmentation_matrix: Option<SegmentationMatrix>,
    pub usage_group_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub product_family_id: ProductFamilyId,
    pub product_id: Option<ProductId>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_owned(Subscription)]
pub struct SubscriptionEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[map(id)]
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub customer_alias: Option<String>,
    pub customer_name: String,
    pub billing_day_anchor: u16,
    pub currency: String,
    pub trial_duration: Option<u32>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub billing_start_date: Option<NaiveDate>,
    pub plan_id: PlanId,
    pub plan_name: String,
    pub plan_version_id: PlanVersionId,
    pub version: u32,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub net_terms: u32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub mrr_cents: u64,
    pub period: BillingPeriodEnum,
    pub status: SubscriptionStatusEnum,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_ref(Invoice)]
pub struct InvoiceEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[map(@.id)]
    pub invoice_id: InvoiceId,
    #[map(@.status.clone())]
    pub status: InvoiceStatusEnum,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    #[map(@.currency.clone())]
    pub currency: String,
    pub tax_amount: i64,
    pub total: i64,
    #[map(@.created_at.clone())]
    pub created_at: NaiveDateTime,
    #[map(@.conn_meta.clone())]
    pub conn_meta: Option<ConnectionMeta>,
    pub amount_due: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoicePdfGeneratedEvent {
    pub id: EventId,
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub pdf_id: StoredDocumentId,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_ref(CreditNote)]
pub struct CreditNoteEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[map(@.id)]
    pub credit_note_id: CreditNoteId,
    #[map(@.status.clone())]
    pub status: CreditNoteStatus,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub invoice_id: InvoiceId,
    pub subscription_id: Option<SubscriptionId>,
    #[map(@.currency.clone())]
    pub currency: String,
    pub tax_amount: i64,
    pub total: i64,
    pub refunded_amount_cents: i64,
    pub credited_amount_cents: i64,
    #[map(@.created_at.clone())]
    pub created_at: NaiveDateTime,
    #[map(@.conn_meta.clone())]
    pub conn_meta: Option<ConnectionMeta>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[map_owned(PaymentTransaction)]
pub struct PaymentTransactionEvent {
    #[ghost(EventId::new())]
    pub id: EventId,
    #[map(id)]
    pub payment_transaction_id: PaymentTransactionId,
    pub tenant_id: TenantId,
    pub invoice_id: InvoiceId,
    pub provider_transaction_id: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub refunded_at: Option<NaiveDateTime>,
    pub amount: i64,
    pub currency: String,
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    pub status: PaymentStatusEnum,
    pub payment_type: PaymentTypeEnum,
    pub error_type: Option<String>,
    pub receipt_pdf_id: Option<StoredDocumentId>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_owned(Quote)]
pub struct QuoteAcceptedEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[map(id)]
    pub quote_id: QuoteId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub create_subscription_on_acceptance: bool,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteConvertedEvent {
    pub id: EventId,
    pub quote_id: QuoteId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
}

impl QuoteConvertedEvent {
    pub fn new(
        quote_id: QuoteId,
        tenant_id: TenantId,
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
    ) -> Self {
        Self {
            id: EventId::new(),
            quote_id,
            tenant_id,
            customer_id,
            subscription_id,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OutboxPgmqHeaders {
    pub event_type: EventType,
}

json_value_serde!(OutboxPgmqHeaders);

impl TryInto<OutboxPgmqHeaders> for &common_domain::pgmq::Headers {
    type Error = StoreErrorReport;
    fn try_into(self) -> Result<OutboxPgmqHeaders, Self::Error> {
        let headers = &self.0;
        headers.try_into()
    }
}

impl TryInto<OutboxEvent> for PgmqMessage {
    type Error = StoreErrorReport;
    fn try_into(self) -> Result<OutboxEvent, Self::Error> {
        let payload = self
            .message
            .ok_or(StoreError::ValueNotFound("Pgmq message".to_string()))?
            .0;

        payload.try_into()
    }
}

impl TryInto<OutboxEvent> for &PgmqMessage {
    type Error = StoreErrorReport;
    fn try_into(self) -> Result<OutboxEvent, Self::Error> {
        let payload = &self
            .message
            .as_ref()
            .ok_or(StoreError::ValueNotFound("Pgmq message".to_string()))?
            .0;

        payload.try_into()
    }
}

impl TryInto<common_domain::pgmq::Headers> for OutboxEvent {
    type Error = StoreErrorReport;
    fn try_into(self) -> Result<common_domain::pgmq::Headers, Self::Error> {
        let headers = OutboxPgmqHeaders {
            event_type: self.event_type(),
        };

        Ok(common_domain::pgmq::Headers(headers.try_into()?))
    }
}

impl TryInto<PgmqMessageRowNew> for OutboxEvent {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<PgmqMessageRowNew, Self::Error> {
        let message = Some(common_domain::pgmq::Message(self.payload_json()?));
        let headers = Some(self.try_into()?);
        Ok(PgmqMessageRowNew { message, headers })
    }
}

impl TryInto<PgmqMessageNew> for OutboxEvent {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<PgmqMessageNew, Self::Error> {
        let message = Some(common_domain::pgmq::Message(self.payload_json()?));
        let headers = Some(self.try_into()?);
        Ok(PgmqMessageNew { message, headers })
    }
}
