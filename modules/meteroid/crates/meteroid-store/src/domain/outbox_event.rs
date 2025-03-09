use crate::StoreResult;
use crate::domain::enums::{BillingPeriodEnum, InvoiceStatusEnum};
use crate::domain::{Address, Customer, DetailedInvoice, ShippingAddress, Subscription};
use crate::errors::{StoreError, StoreErrorReport};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    BaseId, CustomerId, EventId, InvoiceId, PlanId, SubscriptionId, TenantId,
};
use diesel_models::outbox_event::OutboxEventRowNew;
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use strum::Display;
use uuid::Uuid;

pub struct OutboxEvent {
    pub tenant_id: TenantId,
    pub aggregate_id: Uuid,
    pub event_type: EventType,
}

impl OutboxEvent {
    pub fn customer_created(event: CustomerEvent) -> OutboxEvent {
        OutboxEvent {
            tenant_id: event.tenant_id,
            aggregate_id: event.id.as_uuid(),
            event_type: EventType::CustomerCreated(Box::new(event)),
        }
    }

    pub fn invoice_pdf_requested(tenant_id: TenantId, invoice_id: Uuid) -> OutboxEvent {
        OutboxEvent {
            tenant_id,
            aggregate_id: invoice_id,
            event_type: EventType::InvoicePdfRequested,
        }
    }

    pub fn invoice_created(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent {
            tenant_id: event.tenant_id,
            aggregate_id: event.id.as_uuid(),
            event_type: EventType::InvoiceCreated(Box::new(event)),
        }
    }

    pub fn invoice_finalized(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent {
            tenant_id: event.tenant_id,
            aggregate_id: event.id.as_uuid(),
            event_type: EventType::InvoiceFinalized(Box::new(event)),
        }
    }

    pub fn subscription_created(event: SubscriptionEvent) -> OutboxEvent {
        OutboxEvent {
            tenant_id: event.tenant_id,
            aggregate_id: event.id.as_uuid(),
            event_type: EventType::SubscriptionCreated(Box::new(event)),
        }
    }

    fn payload_json(&self) -> StoreResult<Option<serde_json::Value>> {
        match &self.event_type {
            EventType::CustomerCreated(event) => Ok(Some(Self::event_json(event)?)),
            EventType::InvoiceCreated(event) => Ok(Some(Self::event_json(event)?)),
            EventType::InvoiceFinalized(event) => Ok(Some(Self::event_json(event)?)),
            EventType::InvoicePdfRequested => Ok(None),
            EventType::SubscriptionCreated(event) => Ok(Some(Self::event_json(event)?)),
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

#[derive(Display)]
pub enum EventType {
    #[strum(serialize = "customer.created")]
    CustomerCreated(Box<CustomerEvent>),
    #[strum(serialize = "invoice.created")]
    InvoiceCreated(Box<InvoiceEvent>),
    #[strum(serialize = "invoice.finalized")]
    InvoiceFinalized(Box<InvoiceEvent>),
    #[strum(serialize = "invoice.pdf.requested")]
    InvoicePdfRequested,
    #[strum(serialize = "subscription.created")]
    SubscriptionCreated(Box<SubscriptionEvent>),
}

impl EventType {
    pub fn aggregate_type(&self) -> String {
        match self {
            EventType::CustomerCreated(_) => "customer".to_string(),
            EventType::InvoiceCreated(_) => "invoice".to_string(),
            EventType::InvoiceFinalized(_) => "invoice".to_string(),
            EventType::InvoicePdfRequested => "invoice".to_string(),
            EventType::SubscriptionCreated(_) => "subscription".to_string(),
        }
    }
}

impl TryInto<OutboxEventRowNew> for OutboxEvent {
    type Error = StoreErrorReport;
    fn try_into(self) -> Result<OutboxEventRowNew, Self::Error> {
        Ok(OutboxEventRowNew {
            id: EventId::new(),
            tenant_id: self.tenant_id,
            aggregate_id: self.aggregate_id.to_string(),
            aggregate_type: self.event_type.aggregate_type(),
            event_type: self.event_type.to_string(),
            payload: self.payload_json()?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize, o2o)]
#[from_owned(Customer)]
pub struct CustomerEvent {
    pub id: CustomerId,
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
}

#[derive(Debug, Serialize, Deserialize, o2o)]
#[from_owned(Subscription)]
pub struct SubscriptionEvent {
    pub id: SubscriptionId,
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

#[derive(Debug, Serialize, Deserialize, o2o)]
#[from_owned(DetailedInvoice)]
pub struct InvoiceEvent {
    #[map(@.invoice.id)]
    pub id: InvoiceId,
    #[map(@.invoice.status)]
    pub status: InvoiceStatusEnum,
    #[map(@.invoice.tenant_id)]
    pub tenant_id: TenantId,
    #[map(@.invoice.customer_id)]
    pub customer_id: CustomerId,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[map(@.invoice.subscription_id)]
    pub subscription_id: Option<SubscriptionId>,
    #[map(@.invoice.currency)]
    pub currency: String,
    #[map(@.invoice.tax_amount)]
    pub tax_amount: i64,
    #[map(@.invoice.total)]
    pub total: i64,
    #[map(@.invoice.created_at)]
    pub created_at: NaiveDateTime,
}
