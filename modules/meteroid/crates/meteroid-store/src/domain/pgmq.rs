use crate::domain::outbox_event::{CustomerEvent, QuoteAcceptedEvent, SubscriptionEvent};
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    CreditNoteId, CustomerId, CustomerPaymentMethodId, InvoiceId, InvoicingEntityId, PlanVersionId,
    QuoteId, StoredDocumentId, SubscriptionId, TenantId,
};
use common_domain::pgmq::{Headers, Message, MessageId, ReadCt};
use diesel_models::pgmq::{PgmqMessageRow, PgmqMessageRowNew};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use strum::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum PgmqQueue {
    OutboxEvent,
    InvoicePdfRequest,
    CreditNotePdfRequest,
    WebhookOut,
    HubspotSync,
    PennylaneSync,
    InvoiceOrchestration,
    PaymentRequest,
    SendEmailRequest,
    QuoteConversion,
    BiAggregation,
}

impl PgmqQueue {
    pub fn as_str(&self) -> &'static str {
        match self {
            PgmqQueue::OutboxEvent => "outbox_event",
            PgmqQueue::InvoicePdfRequest => "invoice_pdf_request",
            PgmqQueue::CreditNotePdfRequest => "credit_note_pdf_request",
            PgmqQueue::WebhookOut => "webhook_out",
            PgmqQueue::HubspotSync => "hubspot_sync",
            PgmqQueue::PennylaneSync => "pennylane_sync",
            PgmqQueue::InvoiceOrchestration => "invoice_orchestration",
            PgmqQueue::PaymentRequest => "payment_request",
            PgmqQueue::SendEmailRequest => "send_email_request",
            PgmqQueue::QuoteConversion => "quote_conversion",
            PgmqQueue::BiAggregation => "bi_aggregation",
        }
    }
}

impl std::str::FromStr for PgmqQueue {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "outbox_event" => Ok(PgmqQueue::OutboxEvent),
            "invoice_pdf_request" => Ok(PgmqQueue::InvoicePdfRequest),
            "credit_note_pdf_request" => Ok(PgmqQueue::CreditNotePdfRequest),
            "webhook_out" => Ok(PgmqQueue::WebhookOut),
            "hubspot_sync" => Ok(PgmqQueue::HubspotSync),
            "pennylane_sync" => Ok(PgmqQueue::PennylaneSync),
            "invoice_orchestration" => Ok(PgmqQueue::InvoiceOrchestration),
            "payment_request" => Ok(PgmqQueue::PaymentRequest),
            "send_email_request" => Ok(PgmqQueue::SendEmailRequest),
            "quote_conversion" => Ok(PgmqQueue::QuoteConversion),
            "bi_aggregation" => Ok(PgmqQueue::BiAggregation),
            _ => Err(format!("Unknown queue: {s}")),
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
    pub enqueued_at: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct PgmqMessageNew {
    pub message: Option<Message>,
    pub headers: Option<Headers>,
    pub tenant_id: Option<TenantId>,
}

impl From<PgmqMessageNew> for PgmqMessageRowNew {
    fn from(val: PgmqMessageNew) -> PgmqMessageRowNew {
        let headers = merge_tenant_into_headers(val.headers, val.tenant_id);
        PgmqMessageRowNew {
            message: val.message,
            headers,
        }
    }
}

fn merge_tenant_into_headers(
    existing: Option<Headers>,
    tenant_id: Option<TenantId>,
) -> Option<Headers> {
    let tid = match tenant_id {
        Some(tid) => tid,
        None => return existing,
    };

    let mut obj = match existing {
        Some(Headers(serde_json::Value::Object(map))) => map,
        Some(Headers(val)) => {
            let mut map = serde_json::Map::new();
            map.insert("_original".to_string(), val);
            map
        }
        None => serde_json::Map::new(),
    };

    obj.insert(
        "tenant_id".to_string(),
        serde_json::Value::String(tid.to_string()),
    );

    Some(Headers(serde_json::Value::Object(obj)))
}

/// Extract tenant_id from PGMQ message headers (set at enqueue time).
pub fn extract_tenant_id_from_headers(headers: &Option<serde_json::Value>) -> Option<TenantId> {
    let s = headers.as_ref()?.get("tenant_id")?.as_str()?;
    TenantId::from_str(s).ok()
}

/// Macro to implement conversions for a pgmq message type.
/// Use `derive_pgmq_message!(Type, tenant_id)` when `.tenant_id()` returns `TenantId`.
/// Use `derive_pgmq_message!(Type, opt_tenant_id)` when `.tenant_id()` returns `Option<TenantId>`.
/// Use `derive_pgmq_message!(Type)` when there's no tenant_id.
macro_rules! derive_pgmq_message {
    ($type:ty, tenant_id) => {
        impl TryInto<PgmqMessageNew> for $type {
            type Error = StoreErrorReport;
            fn try_into(self) -> Result<PgmqMessageNew, Self::Error> {
                let tenant_id = Some(self.tenant_id());
                Ok(PgmqMessageNew {
                    message: Some(Message(self.try_into()?)),
                    headers: None,
                    tenant_id,
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
    ($type:ty, opt_tenant_id) => {
        impl TryInto<PgmqMessageNew> for $type {
            type Error = StoreErrorReport;
            fn try_into(self) -> Result<PgmqMessageNew, Self::Error> {
                let tenant_id = self.tenant_id();
                Ok(PgmqMessageNew {
                    message: Some(Message(self.try_into()?)),
                    headers: None,
                    tenant_id,
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
    ($type:ty) => {
        impl TryInto<PgmqMessageNew> for $type {
            type Error = StoreErrorReport;
            fn try_into(self) -> Result<PgmqMessageNew, Self::Error> {
                Ok(PgmqMessageNew {
                    message: Some(Message(self.try_into()?)),
                    headers: None,
                    tenant_id: None,
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

    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }
}
json_value_serde!(PaymentRequestEvent);
derive_pgmq_message!(PaymentRequestEvent, tenant_id);

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
        tenant_id: TenantId,
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
    PaymentReminder {
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    },

    PaymentRejected {
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        invoice_pdf_url: String,
        receipt_pdf_url: Option<String>, // or tx details ?
    },

    /// Quote sent to recipient for signature
    QuoteReady {
        tenant_id: TenantId,
        quote_id: QuoteId,
        invoicing_entity_id: InvoicingEntityId,
        quote_number: String,
        expires_at: Option<NaiveDate>,
        company_name: String,
        logo_attachment_id: Option<StoredDocumentId>,
        /// Recipients to send the quote to
        recipient_emails: Vec<String>,
        /// Portal URL for signing the quote
        portal_url: String,
        /// Optional custom message from sender
        custom_message: Option<String>,
        /// Currency of the quote
        currency: String,
    },
}
impl SendEmailRequest {
    pub fn tenant_id(&self) -> TenantId {
        match self {
            SendEmailRequest::InvoiceReady { tenant_id, .. }
            | SendEmailRequest::InvoicePaid { tenant_id, .. }
            | SendEmailRequest::PaymentReminder { tenant_id, .. }
            | SendEmailRequest::PaymentRejected { tenant_id, .. }
            | SendEmailRequest::QuoteReady { tenant_id, .. } => *tenant_id,
        }
    }
}
json_value_serde!(SendEmailRequest);
derive_pgmq_message!(SendEmailRequest, tenant_id);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoicePdfRequestEvent {
    #[serde(default)]
    pub tenant_id: Option<TenantId>,
    pub invoice_id: InvoiceId,
    pub is_accounting: bool,
}

impl InvoicePdfRequestEvent {
    pub fn new(tenant_id: TenantId, invoice_id: InvoiceId, is_accounting: bool) -> Self {
        Self {
            tenant_id: Some(tenant_id),
            invoice_id,
            is_accounting,
        }
    }

    pub fn tenant_id(&self) -> Option<TenantId> {
        self.tenant_id
    }
}
json_value_serde!(InvoicePdfRequestEvent);
derive_pgmq_message!(InvoicePdfRequestEvent, opt_tenant_id);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditNotePdfRequestEvent {
    #[serde(default)]
    pub tenant_id: Option<TenantId>,
    pub credit_note_id: CreditNoteId,
}

impl CreditNotePdfRequestEvent {
    pub fn new(tenant_id: TenantId, credit_note_id: CreditNoteId) -> Self {
        Self {
            tenant_id: Some(tenant_id),
            credit_note_id,
        }
    }

    pub fn tenant_id(&self) -> Option<TenantId> {
        self.tenant_id
    }
}
json_value_serde!(CreditNotePdfRequestEvent);
derive_pgmq_message!(CreditNotePdfRequestEvent, opt_tenant_id);

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
derive_pgmq_message!(HubspotSyncRequestEvent, tenant_id);

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
derive_pgmq_message!(PennylaneSyncRequestEvent, tenant_id);

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
pub enum QuoteConversionRequestEvent {
    QuoteAccepted(Box<QuoteAcceptedEvent>),
}

impl QuoteConversionRequestEvent {
    pub fn tenant_id(&self) -> TenantId {
        match self {
            QuoteConversionRequestEvent::QuoteAccepted(event) => event.tenant_id,
        }
    }
}
json_value_serde!(QuoteConversionRequestEvent);
derive_pgmq_message!(QuoteConversionRequestEvent, tenant_id);

/// BI aggregation events for revenue and customer YTD tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BiAggregationEvent {
    InvoiceFinalized(Box<BiInvoiceFinalizedEvent>),
    CreditNoteFinalized(Box<BiCreditNoteFinalizedEvent>),
}

impl BiAggregationEvent {
    pub fn tenant_id(&self) -> TenantId {
        match self {
            BiAggregationEvent::InvoiceFinalized(event) => event.tenant_id,
            BiAggregationEvent::CreditNoteFinalized(event) => event.tenant_id,
        }
    }
}
json_value_serde!(BiAggregationEvent);
derive_pgmq_message!(BiAggregationEvent, tenant_id);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiInvoiceFinalizedEvent {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: Option<PlanVersionId>,
    pub currency: String,
    pub amount_due: i64,
    pub finalized_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiCreditNoteFinalizedEvent {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: Option<PlanVersionId>,
    pub currency: String,
    pub refunded_amount_cents: i64,
    pub finalized_at: NaiveDateTime,
}
