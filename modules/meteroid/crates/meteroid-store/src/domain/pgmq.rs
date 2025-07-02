use crate::domain::outbox_event::{BillableMetricEvent, CustomerEvent, SubscriptionEvent};
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use chrono::NaiveDate;
use common_domain::ids::{
    CustomerId, CustomerPaymentMethodId, InvoiceId, InvoicingEntityId, StoredDocumentId,
    SubscriptionId, TenantId,
};
use common_domain::pgmq::{Headers, Message, MessageId, ReadCt};
use diesel_models::pgmq::{PgmqMessageRow, PgmqMessageRowNew};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use strum::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum PgmqQueue {
    OutboxEvent,
    InvoicePdfRequest,
    WebhookOut,
    BillableMetricSync,
    HubspotSync,
    PennylaneSync,
    InvoiceOrchestration,
    PaymentRequest,
    SendEmailRequest,
}

impl PgmqQueue {
    pub fn as_str(&self) -> &'static str {
        match self {
            PgmqQueue::OutboxEvent => "outbox_event",
            PgmqQueue::InvoicePdfRequest => "invoice_pdf_request",
            PgmqQueue::WebhookOut => "webhook_out",
            PgmqQueue::BillableMetricSync => "billable_metric_sync",
            PgmqQueue::HubspotSync => "hubspot_sync",
            PgmqQueue::PennylaneSync => "pennylane_sync",

            PgmqQueue::InvoiceOrchestration => "invoice_orchestration",
            PgmqQueue::PaymentRequest => "payment_request",
            PgmqQueue::SendEmailRequest => "send_email_request",
        }
    }
}

#[derive(Debug, Clone, o2o)]
#[from_owned(PgmqMessageRow)]
pub struct PgmqMessage {
    pub msg_id: MessageId,
    pub message: Option<Message>,
    pub headers: Option<Headers>,
    pub read_ct: ReadCt,
}

#[derive(Debug, Clone, o2o)]
#[owned_into(PgmqMessageRowNew)]
pub struct PgmqMessageNew {
    pub message: Option<Message>,
    pub headers: Option<Headers>,
}

/// Macro to implement PgmqEvent and json_value_serde for a type
macro_rules! derive_pgmq_message {
    ($type:ty) => {
        impl TryInto<PgmqMessageNew> for $type {
            type Error = StoreErrorReport;
            fn try_into(self) -> Result<PgmqMessageNew, Self::Error> {
                Ok(PgmqMessageNew {
                    message: Some(Message(self.try_into()?)),
                    headers: None,
                })
            }
        }

        impl TryInto<$type> for &PgmqMessage {
            type Error = StoreErrorReport;
            fn try_into(self) -> Result<$type, Self::Error> {
                let payload = &self
                    .message
                    .as_ref()
                    .ok_or(StoreError::ValueNotFound("Pgmq message".to_string()))?
                    .0;
                payload.try_into()
            }
        }
    };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentRequestEvent {
    pub tenant_id: TenantId,
    pub invoice_id: InvoiceId,
    pub payment_method_id: CustomerPaymentMethodId,
}

impl PaymentRequestEvent {
    pub fn new(
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
    ) -> Self {
        Self {
            tenant_id,
            invoice_id,
            payment_method_id,
        }
    }
}
json_value_serde!(PaymentRequestEvent);
derive_pgmq_message!(PaymentRequestEvent);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SendEmailRequest {
    InvoiceReady {
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        invoice_id: InvoiceId,
        invoice_number: String,
        invoice_date: NaiveDate,
        invoice_due_date: NaiveDate,
        label: String,
        amount_due: i64,
        currency: String,
        company_name: String,
        logo_attachment_id: Option<StoredDocumentId>,
        invoicing_emails: Vec<String>,
        invoice_pdf_id: StoredDocumentId,
    },

    InvoicePaid {
        invoice_id: InvoiceId,
        invoicing_entity_id: InvoicingEntityId,
        invoice_number: String,
        invoice_date: NaiveDate,
        invoice_due_date: NaiveDate,
        label: String,
        amount_paid: i64,
        currency: String,
        company_name: String,
        logo_attachment_id: Option<StoredDocumentId>,
        invoicing_emails: Vec<String>,
        invoice_pdf_id: StoredDocumentId,
        receipt_pdf_id: Option<StoredDocumentId>,
        // lines : Vec<InvoiceLine>, TODO
    },

    /// check once a day, then
    PaymentReminder { invoice_id: InvoiceId },

    PaymentRejected {
        invoice_id: InvoiceId,
        invoice_pdf_url: String,
        receipt_pdf_url: Option<String>, // or tx details ?
    },
}
json_value_serde!(SendEmailRequest);
derive_pgmq_message!(SendEmailRequest);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoicePdfRequestEvent {
    pub invoice_id: InvoiceId,
    pub is_accounting: bool,
}

impl InvoicePdfRequestEvent {
    pub fn new(invoice_id: InvoiceId, is_accounting: bool) -> Self {
        Self {
            invoice_id,
            is_accounting,
        }
    }
}
json_value_serde!(InvoicePdfRequestEvent);
derive_pgmq_message!(InvoicePdfRequestEvent);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HubspotSyncRequestEvent {
    /// sync customer and everything related to it
    CustomerDomain(Box<HubspotSyncCustomerDomain>),
    /// sync subscription
    Subscription(Box<HubspotSyncSubscription>),
    /// sync event generated by an outbox event
    CustomerOutbox(Box<CustomerEvent>),
    /// sync event generated by an outbox event
    SubscriptionOutbox(Box<SubscriptionEvent>),
    /// sync custom properties
    CustomProperties(TenantId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubspotSyncCustomerDomain {
    pub id: CustomerId,
    pub tenant_id: TenantId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubspotSyncSubscription {
    pub id: SubscriptionId,
    pub tenant_id: TenantId,
}

impl HubspotSyncRequestEvent {
    pub fn tenant_id(&self) -> TenantId {
        match self {
            HubspotSyncRequestEvent::CustomerDomain(data) => data.tenant_id,
            HubspotSyncRequestEvent::Subscription(data) => data.tenant_id,
            HubspotSyncRequestEvent::CustomerOutbox(event) => event.tenant_id,
            HubspotSyncRequestEvent::SubscriptionOutbox(event) => event.tenant_id,
            HubspotSyncRequestEvent::CustomProperties(tenant_id) => *tenant_id,
        }
    }
}
json_value_serde!(HubspotSyncRequestEvent);
derive_pgmq_message!(HubspotSyncRequestEvent);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PennylaneSyncRequestEvent {
    /// sync customer
    Customer(Box<PennylaneSyncCustomer>),
    /// sync invoice
    Invoice(Box<PennylaneSyncInvoice>),
    /// sync event generated by an outbox event
    CustomerOutbox(Box<CustomerEvent>),
}

impl PennylaneSyncRequestEvent {
    pub fn tenant_id(&self) -> TenantId {
        match self {
            PennylaneSyncRequestEvent::CustomerOutbox(event) => event.tenant_id,
            PennylaneSyncRequestEvent::Customer(event) => event.tenant_id,
            PennylaneSyncRequestEvent::Invoice(event) => event.tenant_id,
        }
    }
}
json_value_serde!(PennylaneSyncRequestEvent);
derive_pgmq_message!(PennylaneSyncRequestEvent);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PennylaneSyncCustomer {
    pub id: CustomerId,
    pub tenant_id: TenantId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PennylaneSyncInvoice {
    pub id: InvoiceId,
    pub tenant_id: TenantId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BillableMetricSyncRequestEvent {
    BillableMetricCreated(Box<BillableMetricEvent>),
}

impl BillableMetricSyncRequestEvent {
    pub fn tenant_id(&self) -> TenantId {
        match self {
            BillableMetricSyncRequestEvent::BillableMetricCreated(event) => event.tenant_id,
        }
    }
}
json_value_serde!(BillableMetricSyncRequestEvent);
derive_pgmq_message!(BillableMetricSyncRequestEvent);
