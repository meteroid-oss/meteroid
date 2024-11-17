use crate::domain::{Address, Customer, ShippingAddress};
use crate::errors::{StoreError, StoreErrorReport};
use crate::StoreResult;
use diesel_models::outbox_event::OutboxEventRowNew;
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use strum::Display;
use uuid::Uuid;

pub struct OutboxEvent {
    pub tenant_id: Uuid,
    pub aggregate_id: Uuid,
    pub event_type: EventType,
}

impl OutboxEvent {
    pub fn customer_created(event: CustomerCreatedEvent) -> OutboxEvent {
        OutboxEvent {
            tenant_id: event.tenant_id,
            aggregate_id: event.id,
            event_type: EventType::CustomerCreated(Box::new(event)),
        }
    }

    pub fn payload_json(&self) -> StoreResult<Option<serde_json::Value>> {
        match &self.event_type {
            EventType::CustomerCreated(event) => Ok(Some(Self::event_json(event)?)),
            EventType::InvoiceFinalized => Ok(None),
            EventType::InvoicePdfRequested => Ok(None),
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
    CustomerCreated(Box<CustomerCreatedEvent>),
    #[strum(serialize = "invoice.finalized")]
    InvoiceFinalized,
    #[strum(serialize = "invoice.pdf.requested")]
    InvoicePdfRequested,
}

impl EventType {
    pub fn aggregate_type(&self) -> String {
        match self {
            EventType::CustomerCreated(_) => "customer".to_string(),
            EventType::InvoiceFinalized => "invoice".to_string(),
            EventType::InvoicePdfRequested => "invoice".to_string(),
        }
    }
}

impl TryInto<OutboxEventRowNew> for OutboxEvent {
    type Error = StoreErrorReport;
    fn try_into(self) -> Result<OutboxEventRowNew, Self::Error> {
        Ok(OutboxEventRowNew {
            id: Uuid::now_v7(),
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
pub struct CustomerCreatedEvent {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
}
