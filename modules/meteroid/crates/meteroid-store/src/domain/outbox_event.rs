use crate::domain::connectors::ConnectionMeta;
use crate::domain::coupons::CouponDiscount;
use crate::domain::enums::CreditNoteStatus;
use crate::domain::enums::FeeTypeEnum;
use crate::domain::enums::{BillingPeriodEnum, InvoiceStatusEnum, PlanStatusEnum, PlanTypeEnum};
use crate::domain::pgmq::{PgmqMessage, PgmqMessageNew};
use crate::domain::{
    Address, BillableMetric, BillingMetricAggregateEnum, CreditNote, Customer, Invoice,
    PaymentStatusEnum, PaymentTransaction, PaymentTypeEnum, Quote, SegmentationMatrix,
    ShippingAddress, Subscription, SubscriptionStatusEnum, UnitConversionRoundingEnum,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::{StoreResult, json_value_serde};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    AddOnId, BaseId, BillableMetricId, CheckoutSessionId, ConnectorId, CouponId, CreditNoteId,
    CustomerId, CustomerPaymentMethodId, EventId, InvoiceId, PaymentTransactionId, PlanId,
    PlanVersionId, PriceId, ProductFamilyId, ProductId, QuoteId, StoredDocumentId, SubscriptionId,
    TenantId,
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
    PlanCreated(Box<PlanEvent>),
    PlanPublished(Box<PlanEvent>),
    PlanArchived(Box<PlanEvent>),
    ProductCreated(Box<ProductEvent>),
    ProductUpdated(Box<ProductEvent>),
    ProductArchived(Box<ProductEvent>),
    BillableMetricUpdated(Box<BillableMetricEvent>),
    BillableMetricArchived(Box<BillableMetricEvent>),
    CouponCreated(Box<CouponEvent>),
    CouponUpdated(Box<CouponEvent>),
    CouponArchived(Box<CouponEvent>),
    AddOnCreated(Box<AddOnEvent>),
    AddOnUpdated(Box<AddOnEvent>),
    AddOnArchived(Box<AddOnEvent>),
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
    PlanCreated,
    PlanPublished,
    PlanArchived,
    ProductCreated,
    ProductUpdated,
    ProductArchived,
    BillableMetricUpdated,
    BillableMetricArchived,
    CouponCreated,
    CouponUpdated,
    CouponArchived,
    AddOnCreated,
    AddOnUpdated,
    AddOnArchived,
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
            OutboxEvent::PlanCreated(event) => event.id,
            OutboxEvent::PlanPublished(event) => event.id,
            OutboxEvent::PlanArchived(event) => event.id,
            OutboxEvent::ProductCreated(event) => event.id,
            OutboxEvent::ProductUpdated(event) => event.id,
            OutboxEvent::ProductArchived(event) => event.id,
            OutboxEvent::BillableMetricUpdated(event) => event.id,
            OutboxEvent::BillableMetricArchived(event) => event.id,
            OutboxEvent::CouponCreated(event) => event.id,
            OutboxEvent::CouponUpdated(event) => event.id,
            OutboxEvent::CouponArchived(event) => event.id,
            OutboxEvent::AddOnCreated(event) => event.id,
            OutboxEvent::AddOnUpdated(event) => event.id,
            OutboxEvent::AddOnArchived(event) => event.id,
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
            OutboxEvent::PlanCreated(event) => event.tenant_id,
            OutboxEvent::PlanPublished(event) => event.tenant_id,
            OutboxEvent::PlanArchived(event) => event.tenant_id,
            OutboxEvent::ProductCreated(event) => event.tenant_id,
            OutboxEvent::ProductUpdated(event) => event.tenant_id,
            OutboxEvent::ProductArchived(event) => event.tenant_id,
            OutboxEvent::BillableMetricUpdated(event) => event.tenant_id,
            OutboxEvent::BillableMetricArchived(event) => event.tenant_id,
            OutboxEvent::CouponCreated(event) => event.tenant_id,
            OutboxEvent::CouponUpdated(event) => event.tenant_id,
            OutboxEvent::CouponArchived(event) => event.tenant_id,
            OutboxEvent::AddOnCreated(event) => event.tenant_id,
            OutboxEvent::AddOnUpdated(event) => event.tenant_id,
            OutboxEvent::AddOnArchived(event) => event.tenant_id,
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
            OutboxEvent::PlanCreated(event) => event.plan_id.as_uuid(),
            OutboxEvent::PlanPublished(event) => event.plan_id.as_uuid(),
            OutboxEvent::PlanArchived(event) => event.plan_id.as_uuid(),
            OutboxEvent::ProductCreated(event) => event.product_id.as_uuid(),
            OutboxEvent::ProductUpdated(event) => event.product_id.as_uuid(),
            OutboxEvent::ProductArchived(event) => event.product_id.as_uuid(),
            OutboxEvent::BillableMetricUpdated(event) => event.metric_id.as_uuid(),
            OutboxEvent::BillableMetricArchived(event) => event.metric_id.as_uuid(),
            OutboxEvent::CouponCreated(event) => event.coupon_id.as_uuid(),
            OutboxEvent::CouponUpdated(event) => event.coupon_id.as_uuid(),
            OutboxEvent::CouponArchived(event) => event.coupon_id.as_uuid(),
            OutboxEvent::AddOnCreated(event) => event.add_on_id.as_uuid(),
            OutboxEvent::AddOnUpdated(event) => event.add_on_id.as_uuid(),
            OutboxEvent::AddOnArchived(event) => event.add_on_id.as_uuid(),
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
            OutboxEvent::PlanCreated(_) => "Plan".to_string(),
            OutboxEvent::PlanPublished(_) => "Plan".to_string(),
            OutboxEvent::PlanArchived(_) => "Plan".to_string(),
            OutboxEvent::ProductCreated(_) => "Product".to_string(),
            OutboxEvent::ProductUpdated(_) => "Product".to_string(),
            OutboxEvent::ProductArchived(_) => "Product".to_string(),
            OutboxEvent::BillableMetricUpdated(_) => "BillableMetric".to_string(),
            OutboxEvent::BillableMetricArchived(_) => "BillableMetric".to_string(),
            OutboxEvent::CouponCreated(_) => "Coupon".to_string(),
            OutboxEvent::CouponUpdated(_) => "Coupon".to_string(),
            OutboxEvent::CouponArchived(_) => "Coupon".to_string(),
            OutboxEvent::AddOnCreated(_) => "AddOn".to_string(),
            OutboxEvent::AddOnUpdated(_) => "AddOn".to_string(),
            OutboxEvent::AddOnArchived(_) => "AddOn".to_string(),
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
            OutboxEvent::PlanCreated(_) => EventType::PlanCreated,
            OutboxEvent::PlanPublished(_) => EventType::PlanPublished,
            OutboxEvent::PlanArchived(_) => EventType::PlanArchived,
            OutboxEvent::ProductCreated(_) => EventType::ProductCreated,
            OutboxEvent::ProductUpdated(_) => EventType::ProductUpdated,
            OutboxEvent::ProductArchived(_) => EventType::ProductArchived,
            OutboxEvent::BillableMetricUpdated(_) => EventType::BillableMetricUpdated,
            OutboxEvent::BillableMetricArchived(_) => EventType::BillableMetricArchived,
            OutboxEvent::CouponCreated(_) => EventType::CouponCreated,
            OutboxEvent::CouponUpdated(_) => EventType::CouponUpdated,
            OutboxEvent::CouponArchived(_) => EventType::CouponArchived,
            OutboxEvent::AddOnCreated(_) => EventType::AddOnCreated,
            OutboxEvent::AddOnUpdated(_) => EventType::AddOnUpdated,
            OutboxEvent::AddOnArchived(_) => EventType::AddOnArchived,
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

    pub fn plan_created(event: PlanEvent) -> OutboxEvent {
        OutboxEvent::PlanCreated(Box::new(event))
    }

    pub fn plan_published(event: PlanEvent) -> OutboxEvent {
        OutboxEvent::PlanPublished(Box::new(event))
    }

    pub fn plan_archived(event: PlanEvent) -> OutboxEvent {
        OutboxEvent::PlanArchived(Box::new(event))
    }

    pub fn product_created(event: ProductEvent) -> OutboxEvent {
        OutboxEvent::ProductCreated(Box::new(event))
    }

    pub fn product_updated(event: ProductEvent) -> OutboxEvent {
        OutboxEvent::ProductUpdated(Box::new(event))
    }

    pub fn product_archived(event: ProductEvent) -> OutboxEvent {
        OutboxEvent::ProductArchived(Box::new(event))
    }

    pub fn billable_metric_updated(event: BillableMetricEvent) -> OutboxEvent {
        OutboxEvent::BillableMetricUpdated(Box::new(event))
    }

    pub fn billable_metric_archived(event: BillableMetricEvent) -> OutboxEvent {
        OutboxEvent::BillableMetricArchived(Box::new(event))
    }

    pub fn coupon_created(event: CouponEvent) -> OutboxEvent {
        OutboxEvent::CouponCreated(Box::new(event))
    }

    pub fn coupon_updated(event: CouponEvent) -> OutboxEvent {
        OutboxEvent::CouponUpdated(Box::new(event))
    }

    pub fn coupon_archived(event: CouponEvent) -> OutboxEvent {
        OutboxEvent::CouponArchived(Box::new(event))
    }

    pub fn add_on_created(event: AddOnEvent) -> OutboxEvent {
        OutboxEvent::AddOnCreated(Box::new(event))
    }

    pub fn add_on_updated(event: AddOnEvent) -> OutboxEvent {
        OutboxEvent::AddOnUpdated(Box::new(event))
    }

    pub fn add_on_archived(event: AddOnEvent) -> OutboxEvent {
        OutboxEvent::AddOnArchived(Box::new(event))
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
#[ghosts(archived_at: None, updated_at: None)]
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
    pub plan_version_id: Option<PlanVersionId>,
    #[map(@.currency.clone())]
    pub currency: String,
    pub tax_amount: i64,
    pub total: i64,
    #[map(@.created_at.clone())]
    pub created_at: NaiveDateTime,
    #[map(@.conn_meta.clone())]
    pub conn_meta: Option<ConnectionMeta>,
    pub amount_due: i64,
    pub finalized_at: Option<NaiveDateTime>,
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
    pub plan_version_id: Option<PlanVersionId>,
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
    pub finalized_at: Option<NaiveDateTime>,
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
    pub invoice_id: Option<InvoiceId>,
    pub checkout_session_id: Option<CheckoutSessionId>,
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
    pub pending_plan_version_id: Option<PlanVersionId>,
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

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEvent {
    pub id: EventId,
    pub plan_id: PlanId,
    pub plan_version_id: PlanVersionId,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: Option<String>,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub currency: String,
    pub version: i32,
    pub created_at: NaiveDateTime,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductEvent {
    pub id: EventId,
    pub product_id: ProductId,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: Option<String>,
    pub fee_type: FeeTypeEnum,
    pub product_family_id: ProductFamilyId,
    pub created_at: NaiveDateTime,
}

impl ProductEvent {
    pub fn new(
        product_id: ProductId,
        tenant_id: TenantId,
        name: String,
        description: Option<String>,
        fee_type: FeeTypeEnum,
        product_family_id: ProductFamilyId,
        created_at: NaiveDateTime,
    ) -> Self {
        Self {
            id: EventId::new(),
            product_id,
            tenant_id,
            name,
            description,
            fee_type,
            product_family_id,
            created_at,
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouponEvent {
    pub id: EventId,
    pub coupon_id: CouponId,
    pub tenant_id: TenantId,
    pub code: String,
    pub description: String,
    pub discount: CouponDiscount,
    pub expires_at: Option<NaiveDateTime>,
    pub redemption_limit: Option<i32>,
    pub recurring_value: Option<i32>,
    pub reusable: bool,
    pub disabled: bool,
    pub created_at: NaiveDateTime,
}

impl From<crate::domain::coupons::Coupon> for CouponEvent {
    fn from(c: crate::domain::coupons::Coupon) -> Self {
        Self {
            id: EventId::new(),
            coupon_id: c.id,
            tenant_id: c.tenant_id,
            code: c.code,
            description: c.description,
            discount: c.discount,
            expires_at: c.expires_at,
            redemption_limit: c.redemption_limit,
            recurring_value: c.recurring_value,
            reusable: c.reusable,
            disabled: c.disabled,
            created_at: c.created_at,
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddOnEvent {
    pub id: EventId,
    pub add_on_id: AddOnId,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: Option<String>,
    pub product_id: ProductId,
    pub price_id: PriceId,
    pub fee_type: Option<FeeTypeEnum>,
    pub self_serviceable: bool,
    pub max_instances_per_subscription: Option<i32>,
    pub created_at: NaiveDateTime,
}

impl From<crate::domain::add_ons::AddOn> for AddOnEvent {
    fn from(a: crate::domain::add_ons::AddOn) -> Self {
        Self {
            id: EventId::new(),
            add_on_id: a.id,
            tenant_id: a.tenant_id,
            name: a.name,
            description: a.description,
            product_id: a.product_id,
            price_id: a.price_id,
            fee_type: a.fee_type,
            self_serviceable: a.self_serviceable,
            max_instances_per_subscription: a.max_instances_per_subscription,
            created_at: a.created_at,
        }
    }
}

impl PlanEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        plan_id: PlanId,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
        name: String,
        description: Option<String>,
        plan_type: PlanTypeEnum,
        status: PlanStatusEnum,
        currency: String,
        version: i32,
        created_at: NaiveDateTime,
    ) -> Self {
        Self {
            id: EventId::new(),
            plan_id,
            plan_version_id,
            tenant_id,
            name,
            description,
            plan_type,
            status,
            currency,
            version,
            created_at,
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
        let tenant_id = Some(self.tenant_id());
        let message = Some(common_domain::pgmq::Message(self.payload_json()?));
        let headers = Some(self.try_into()?);
        Ok(PgmqMessageNew {
            message,
            headers,
            tenant_id,
        })
    }
}
