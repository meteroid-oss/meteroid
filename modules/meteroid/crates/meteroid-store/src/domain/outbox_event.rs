use crate::domain::connectors::ConnectionMeta;
use crate::domain::enums::{BillingPeriodEnum, InvoiceStatusEnum};
use crate::domain::pgmq::{PgmqMessage, PgmqMessageNew};
use crate::domain::{Address, Customer, DetailedInvoice, Invoice, ShippingAddress, Subscription};
use crate::errors::{StoreError, StoreErrorReport};
use crate::{StoreResult, json_value_serde};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    BankAccountId, BaseId, ConnectorId, CustomerId, EventId, InvoiceId, PlanId, SubscriptionId,
    TenantId,
};
use diesel_models::outbox_event::OutboxEventRowNew;
use diesel_models::pgmq::PgmqMessageRowNew;
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use strum::Display;
use uuid::Uuid;

#[derive(Display, Debug, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum OutboxEvent {
    CustomerCreated(Box<CustomerEvent>),
    InvoiceCreated(Box<InvoiceEvent>),
    InvoiceFinalized(Box<InvoiceEvent>),
    InvoicePdfGenerated(Box<InvoicePdfGeneratedEvent>),
    SubscriptionCreated(Box<SubscriptionEvent>),
}

#[derive(Display, Debug, Serialize, Deserialize)]
pub enum EventType {
    CustomerCreated,
    InvoiceCreated,
    InvoiceFinalized,
    InvoicePdfGenerated,
    SubscriptionCreated,
}

json_value_serde!(OutboxEvent);

impl OutboxEvent {
    pub fn event_id(&self) -> EventId {
        match self {
            OutboxEvent::CustomerCreated(event) => event.id,
            OutboxEvent::InvoiceCreated(event) => event.id,
            OutboxEvent::InvoiceFinalized(event) => event.id,
            OutboxEvent::InvoicePdfGenerated(event) => event.id,
            OutboxEvent::SubscriptionCreated(event) => event.id,
        }
    }

    pub fn tenant_id(&self) -> TenantId {
        match self {
            OutboxEvent::CustomerCreated(event) => event.tenant_id,
            OutboxEvent::InvoiceCreated(event) => event.tenant_id,
            OutboxEvent::InvoiceFinalized(event) => event.tenant_id,
            OutboxEvent::InvoicePdfGenerated(event) => event.tenant_id,
            OutboxEvent::SubscriptionCreated(event) => event.tenant_id,
        }
    }

    pub fn aggregate_id(&self) -> Uuid {
        match self {
            OutboxEvent::CustomerCreated(event) => event.customer_id.as_uuid(),
            OutboxEvent::InvoiceCreated(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoiceFinalized(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoicePdfGenerated(event) => event.invoice_id.as_uuid(),
            OutboxEvent::SubscriptionCreated(event) => event.subscription_id.as_uuid(),
        }
    }

    pub fn aggregate_type(&self) -> String {
        match self {
            OutboxEvent::CustomerCreated(_) => "Customer".to_string(),
            OutboxEvent::InvoiceCreated(_) => "Invoice".to_string(),
            OutboxEvent::InvoiceFinalized(_) => "Invoice".to_string(),
            OutboxEvent::InvoicePdfGenerated(_) => "Invoice".to_string(),
            OutboxEvent::SubscriptionCreated(_) => "Subscription".to_string(),
        }
    }

    pub fn event_type(&self) -> EventType {
        match self {
            OutboxEvent::CustomerCreated(_) => EventType::CustomerCreated,
            OutboxEvent::InvoiceCreated(_) => EventType::InvoiceCreated,
            OutboxEvent::InvoiceFinalized(_) => EventType::InvoiceFinalized,
            OutboxEvent::InvoicePdfGenerated(_) => EventType::InvoicePdfGenerated,
            OutboxEvent::SubscriptionCreated(_) => EventType::SubscriptionCreated,
        }
    }

    pub fn customer_created(event: CustomerEvent) -> OutboxEvent {
        OutboxEvent::CustomerCreated(Box::new(event))
    }

    pub fn invoice_created(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoiceCreated(Box::new(event))
    }

    pub fn invoice_finalized(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoiceFinalized(Box::new(event))
    }

    pub fn invoice_pdf_generated(event: InvoicePdfGeneratedEvent) -> OutboxEvent {
        OutboxEvent::InvoicePdfGenerated(Box::new(event))
    }

    pub fn subscription_created(event: SubscriptionEvent) -> OutboxEvent {
        OutboxEvent::SubscriptionCreated(Box::new(event))
    }

    fn payload_json(&self) -> StoreResult<Option<serde_json::Value>> {
        match self {
            OutboxEvent::CustomerCreated(event) => Ok(Some(Self::event_json(event)?)),
            OutboxEvent::InvoiceCreated(event) => Ok(Some(Self::event_json(event)?)),
            OutboxEvent::InvoiceFinalized(event) => Ok(Some(Self::event_json(event)?)),
            OutboxEvent::InvoicePdfGenerated(event) => Ok(Some(Self::event_json(event)?)),
            OutboxEvent::SubscriptionCreated(event) => Ok(Some(Self::event_json(event)?)),
        }
    }

    fn event_json<T>(event: &T) -> StoreResult<serde_json::Value>
    where
        T: Serialize,
    {
        serde_json::to_value(event).map_err(|e| {
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

#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_owned(Customer)]
pub struct CustomerEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[map(id)]
    pub customer_id: CustomerId,
    pub tenant_id: TenantId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_email: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub invoicing_emails: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shipping_address: Option<ShippingAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vat_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank_account_id: Option<BankAccountId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conn_meta: Option<ConnectionMeta>,
}

impl CustomerEvent {
    pub fn get_pennylane_id(&self, connector_id: ConnectorId) -> Option<i64> {
        self.conn_meta
            .as_ref()
            .and_then(|meta| meta.get_pennylane_id(connector_id))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_owned(Subscription)]
pub struct SubscriptionEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[map(id)]
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_alias: Option<String>,
    pub customer_name: String,
    pub billing_day_anchor: u16,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_duration: Option<u32>,
    pub start_date: NaiveDate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_start_date: Option<NaiveDate>,
    pub plan_id: PlanId,
    pub plan_name: String,
    pub plan_version_id: Uuid,
    pub version: u32,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub net_terms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice_memo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<NaiveDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canceled_at: Option<NaiveDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancellation_reason: Option<String>,
    pub mrr_cents: u64,
    pub period: BillingPeriodEnum,
}

#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_owned(DetailedInvoice)]
#[from_owned(Invoice)]
pub struct InvoiceEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[from(DetailedInvoice| @.invoice.id)]
    #[from(Invoice| @.id)]
    pub invoice_id: InvoiceId,
    #[from(DetailedInvoice| @.invoice.status)]
    pub status: InvoiceStatusEnum,
    #[from(DetailedInvoice| @.invoice.tenant_id)]
    pub tenant_id: TenantId,
    #[from(DetailedInvoice| @.invoice.customer_id)]
    pub customer_id: CustomerId,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[from(DetailedInvoice| @.invoice.subscription_id)]
    pub subscription_id: Option<SubscriptionId>,
    #[from(DetailedInvoice| @.invoice.currency)]
    pub currency: String,
    #[from(DetailedInvoice| @.invoice.tax_amount)]
    pub tax_amount: i64,
    #[from(DetailedInvoice| @.invoice.total)]
    pub total: i64,
    #[from(DetailedInvoice| @.invoice.created_at)]
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoicePdfGeneratedEvent {
    pub id: EventId,
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub pdf_id: Uuid,
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
        let message = self.payload_json()?.map(common_domain::pgmq::Message);
        let headers = Some(self.try_into()?);
        Ok(PgmqMessageRowNew { message, headers })
    }
}

impl TryInto<PgmqMessageNew> for OutboxEvent {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<PgmqMessageNew, Self::Error> {
        let message = self.payload_json()?.map(common_domain::pgmq::Message);
        let headers = Some(self.try_into()?);
        Ok(PgmqMessageNew { message, headers })
    }
}
