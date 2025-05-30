use crate::domain::connectors::ConnectionMeta;
use crate::domain::enums::{BillingPeriodEnum, InvoiceStatusEnum};
use crate::domain::pgmq::{PgmqMessage, PgmqMessageNew};
use crate::domain::{
    Address, BillableMetric, BillingMetricAggregateEnum, Customer, Invoice, SegmentationMatrix,
    ShippingAddress, Subscription, SubscriptionStatusEnum, UnitConversionRoundingEnum,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::{StoreResult, json_value_serde};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    BankAccountId, BaseId, BillableMetricId, ConnectorId, CustomerId, EventId, InvoiceId, PlanId,
    PlanVersionId, ProductFamilyId, ProductId, SubscriptionId, TenantId,
};
use diesel_models::outbox_event::OutboxEventRowNew;
use diesel_models::pgmq::PgmqMessageRowNew;
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use strum::Display;
use uuid::Uuid;

#[derive(Display, Debug, Serialize, Deserialize)]
pub enum OutboxEvent {
    CustomerCreated(Box<CustomerEvent>),
    BillableMetricCreated(Box<BillableMetricEvent>),
    InvoiceCreated(Box<InvoiceEvent>),
    InvoiceFinalized(Box<InvoiceEvent>),
    InvoicePaid(Box<InvoiceEvent>),
    InvoicePdfGenerated(Box<InvoicePdfGeneratedEvent>),
    SubscriptionCreated(Box<SubscriptionEvent>),
}

#[derive(Display, Debug, Serialize, Deserialize)]
pub enum EventType {
    CustomerCreated,
    BillableMetricCreated,
    InvoiceCreated,
    InvoiceFinalized,
    InvoicePaid,
    InvoicePdfGenerated,
    SubscriptionCreated,
}

json_value_serde!(OutboxEvent);

impl OutboxEvent {
    pub fn event_id(&self) -> EventId {
        match self {
            OutboxEvent::CustomerCreated(event) => event.id,
            OutboxEvent::BillableMetricCreated(event) => event.id,
            OutboxEvent::InvoiceCreated(event) => event.id,
            OutboxEvent::InvoiceFinalized(event) => event.id,
            OutboxEvent::InvoicePaid(event) => event.id,
            OutboxEvent::InvoicePdfGenerated(event) => event.id,
            OutboxEvent::SubscriptionCreated(event) => event.id,
        }
    }

    pub fn tenant_id(&self) -> TenantId {
        match self {
            OutboxEvent::CustomerCreated(event) => event.tenant_id,
            OutboxEvent::BillableMetricCreated(event) => event.tenant_id,
            OutboxEvent::InvoiceCreated(event) => event.tenant_id,
            OutboxEvent::InvoiceFinalized(event) => event.tenant_id,
            OutboxEvent::InvoicePaid(event) => event.tenant_id,
            OutboxEvent::InvoicePdfGenerated(event) => event.tenant_id,
            OutboxEvent::SubscriptionCreated(event) => event.tenant_id,
        }
    }

    pub fn aggregate_id(&self) -> Uuid {
        match self {
            OutboxEvent::CustomerCreated(event) => event.customer_id.as_uuid(),
            OutboxEvent::BillableMetricCreated(event) => event.metric_id.as_uuid(),
            OutboxEvent::InvoiceCreated(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoiceFinalized(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoicePaid(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoicePdfGenerated(event) => event.invoice_id.as_uuid(),
            OutboxEvent::SubscriptionCreated(event) => event.subscription_id.as_uuid(),
        }
    }

    pub fn aggregate_type(&self) -> String {
        match self {
            OutboxEvent::CustomerCreated(_) => "Customer".to_string(),
            OutboxEvent::BillableMetricCreated(_) => "BillableMetric".to_string(),
            OutboxEvent::InvoiceCreated(_) => "Invoice".to_string(),
            OutboxEvent::InvoiceFinalized(_) => "Invoice".to_string(),
            OutboxEvent::InvoicePaid(_) => "Invoice".to_string(),
            OutboxEvent::InvoicePdfGenerated(_) => "Invoice".to_string(),
            OutboxEvent::SubscriptionCreated(_) => "Subscription".to_string(),
        }
    }

    pub fn event_type(&self) -> EventType {
        match self {
            OutboxEvent::CustomerCreated(_) => EventType::CustomerCreated,
            OutboxEvent::BillableMetricCreated(_) => EventType::BillableMetricCreated,
            OutboxEvent::InvoiceCreated(_) => EventType::InvoiceCreated,
            OutboxEvent::InvoiceFinalized(_) => EventType::InvoiceFinalized,
            OutboxEvent::InvoicePaid(_) => EventType::InvoicePaid,
            OutboxEvent::InvoicePdfGenerated(_) => EventType::InvoicePdfGenerated,
            OutboxEvent::SubscriptionCreated(_) => EventType::SubscriptionCreated,
        }
    }

    pub fn customer_created(event: CustomerEvent) -> OutboxEvent {
        OutboxEvent::CustomerCreated(Box::new(event))
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

    pub fn invoice_pdf_generated(event: InvoicePdfGeneratedEvent) -> OutboxEvent {
        OutboxEvent::InvoicePdfGenerated(Box::new(event))
    }

    pub fn subscription_created(event: SubscriptionEvent) -> OutboxEvent {
        OutboxEvent::SubscriptionCreated(Box::new(event))
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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

// TODO golden tests
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[map_owned(BillableMetric)]
#[ghosts(archived_at: None, updated_at: None)]
pub struct BillableMetricEvent {
    #[ghost(EventId::new())]
    pub id: EventId,
    #[map(id)]
    pub metric_id: BillableMetricId,
    pub tenant_id: TenantId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub code: String,
    pub aggregation_type: BillingMetricAggregateEnum,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregation_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_conversion_factor: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segmentation_matrix: Option<SegmentationMatrix>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_group_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub product_family_id: ProductFamilyId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_id: Option<ProductId>,
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
    pub plan_version_id: PlanVersionId,
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
    pub mrr_cents: u64,
    pub period: BillingPeriodEnum,
    pub status: SubscriptionStatusEnum,
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
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
