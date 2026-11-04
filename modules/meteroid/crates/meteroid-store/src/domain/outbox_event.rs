use crate::domain::connectors::ConnectionMeta;
use crate::domain::coupons::CouponDiscount;
use crate::domain::einvoicing::{EInvoicingReport, EInvoicingStatus};
use crate::domain::enums::FeeTypeEnum;
use crate::domain::enums::{BillingPeriodEnum, InvoiceStatusEnum, PlanStatusEnum, PlanTypeEnum};
use crate::domain::enums::{CreditNoteStatus, DeclineKind};
use crate::domain::invoice_lines::LineItem;
use crate::domain::invoices::TaxBreakdownItem;
use crate::domain::pgmq::{PgmqMessage, PgmqMessageNew};
use crate::domain::usage_alert_events::is_limit_pct;
pub use crate::domain::usage_alert_events::{
    BudgetAlertEvent, EntitlementAlertEvent, TooManyGroupsEvent,
};
pub use crate::domain::wallet_events::{
    CreditGrantExpiredEvent, TopUpCapPeriod, WalletAutoTopUpCapReachedEvent,
    WalletAutoTopUpConsentChangedEvent, WalletAutoTopUpEvent, WalletAutoTopUpEventMeta,
    WalletBalanceLowEvent, WalletCreditRecoveredEvent, WalletCreditUnrecoveredEvent,
    WalletDebitedEvent, WalletDepletedEvent, WalletFundedEvent, WalletRecoveredEvent,
};
use crate::domain::{
    Address, BillableMetric, BillingMetricAggregateEnum, CreditNote, Customer, Invoice,
    PaymentStatusEnum, PaymentTransaction, PaymentTypeEnum, Quote, RefundMode, ReversalKind,
    SegmentationMatrix, ShippingAddress, Subscription, SubscriptionStatusEnum,
    UnitConversionRoundingEnum,
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
    /// A recurring usage-only period settled with nothing to bill (total 0, no number):
    /// terminal, non-revenue counterpart to `invoice.created` for the closed draft.
    InvoiceClosed(Box<InvoiceEvent>),
    /// A per-subscription draft was merged into a consolidated invoice (the child's
    /// standalone lifecycle ends here; the parent is finalized/charged/paid).
    InvoiceConsolidated(Box<InvoiceEvent>),
    /// A draft was deleted. The payload is the last snapshot of the row — the invoice
    /// no longer exists by the time consumers see this.
    InvoiceDeleted(Box<InvoiceEvent>),
    // only triggered at finalization. Other pdfs (other lang etc) do not trigger this.
    InvoiceAccountingPdfGenerated(Box<InvoicePdfGeneratedEvent>),
    CreditNoteCreated(Box<CreditNoteEvent>),
    CreditNoteFinalized(Box<CreditNoteEvent>),
    CreditNoteVoided(Box<CreditNoteEvent>),
    SubscriptionCreated(Box<SubscriptionEvent>),
    SubscriptionUpdated(Box<SubscriptionEvent>),
    SubscriptionCancelled(Box<SubscriptionEvent>),
    SubscriptionEnded(Box<SubscriptionEvent>),
    PaymentTransactionSaved(Box<PaymentTransactionEvent>),
    /// A refund row was created and is on its way to the provider.
    RefundIssued(Box<PaymentTransactionEvent>),
    RefundSettled(Box<PaymentTransactionEvent>),
    RefundFailed(Box<PaymentTransactionEvent>),
    /// A settled payment was involuntarily clawed back; carries the parent Payment row.
    PaymentReversed(Box<PaymentTransactionEvent>),
    /// Clawed-back money came back (dispute won, chargeback cancelled); carries the Payment row.
    PaymentReinstated(Box<PaymentTransactionEvent>),
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
    WalletFunded(Box<WalletFundedEvent>),
    WalletDebited(Box<WalletDebitedEvent>),
    CreditGrantExpired(Box<CreditGrantExpiredEvent>),
    WalletBalanceLow(Box<WalletBalanceLowEvent>),
    WalletCreditUnrecovered(Box<WalletCreditUnrecoveredEvent>),
    WalletCreditRecovered(Box<WalletCreditRecoveredEvent>),
    WalletAutoTopUpFailed(Box<WalletAutoTopUpEvent>),
    WalletAutoTopUpSuspended(Box<WalletAutoTopUpEvent>),
    WalletAutoTopUpConsentChanged(Box<WalletAutoTopUpConsentChangedEvent>),
    WalletDepleted(Box<WalletDepletedEvent>),
    WalletRecovered(Box<WalletRecoveredEvent>),
    WalletAutoTopUpCapReached(Box<WalletAutoTopUpCapReachedEvent>),
    EntitlementUsageThresholdReached(Box<EntitlementAlertEvent>),
    EntitlementLimitReached(Box<EntitlementAlertEvent>),
    BudgetThresholdReached(Box<BudgetAlertEvent>),
    BudgetExceeded(Box<BudgetAlertEvent>),
    EntitlementTooManyGroups(Box<TooManyGroupsEvent>),
}

#[derive(Display, Debug, Serialize, Deserialize, PartialEq, Clone, Copy, strum::EnumIter)]
pub enum EventType {
    CustomerCreated,
    CustomerUpdated,
    BillableMetricCreated,
    InvoiceCreated,
    InvoiceFinalized,
    InvoicePaid,
    InvoiceVoided,
    InvoiceClosed,
    InvoiceConsolidated,
    InvoiceDeleted,
    InvoiceAccountingPdfGenerated,
    CreditNoteCreated,
    CreditNoteFinalized,
    CreditNoteVoided,
    SubscriptionCreated,
    SubscriptionUpdated,
    SubscriptionCancelled,
    SubscriptionEnded,
    PaymentTransactionReceived,
    RefundIssued,
    RefundSettled,
    RefundFailed,
    PaymentReversed,
    PaymentReinstated,
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
    WalletFunded,
    WalletDebited,
    CreditGrantExpired,
    WalletBalanceLow,
    WalletCreditUnrecovered,
    WalletCreditRecovered,
    WalletAutoTopUpFailed,
    WalletAutoTopUpSuspended,
    WalletAutoTopUpConsentChanged,
    WalletDepleted,
    WalletRecovered,
    WalletAutoTopUpCapReached,
    EntitlementUsageThresholdReached,
    EntitlementLimitReached,
    BudgetThresholdReached,
    BudgetExceeded,
    EntitlementTooManyGroups,
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
            OutboxEvent::InvoiceClosed(event) => event.id,
            OutboxEvent::InvoiceConsolidated(event) => event.id,
            OutboxEvent::InvoiceDeleted(event) => event.id,
            OutboxEvent::InvoiceAccountingPdfGenerated(event) => event.id,
            OutboxEvent::CreditNoteCreated(event) => event.id,
            OutboxEvent::CreditNoteFinalized(event) => event.id,
            OutboxEvent::CreditNoteVoided(event) => event.id,
            OutboxEvent::SubscriptionCreated(event) => event.id,
            OutboxEvent::SubscriptionUpdated(event) => event.id,
            OutboxEvent::SubscriptionCancelled(event) => event.id,
            OutboxEvent::SubscriptionEnded(event) => event.id,
            OutboxEvent::PaymentTransactionSaved(event) => event.id,
            OutboxEvent::RefundIssued(event) => event.id,
            OutboxEvent::RefundSettled(event) => event.id,
            OutboxEvent::RefundFailed(event) => event.id,
            OutboxEvent::PaymentReversed(event) => event.id,
            OutboxEvent::PaymentReinstated(event) => event.id,
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
            OutboxEvent::WalletFunded(event) => event.id,
            OutboxEvent::WalletDebited(event) => event.id,
            OutboxEvent::CreditGrantExpired(event) => event.id,
            OutboxEvent::WalletBalanceLow(event) => event.id,
            OutboxEvent::WalletCreditUnrecovered(event) => event.id,
            OutboxEvent::WalletCreditRecovered(event) => event.id,
            OutboxEvent::WalletAutoTopUpFailed(event) => event.id,
            OutboxEvent::WalletAutoTopUpSuspended(event) => event.id,
            OutboxEvent::WalletAutoTopUpConsentChanged(event) => event.id,
            OutboxEvent::WalletDepleted(event) => event.id,
            OutboxEvent::WalletRecovered(event) => event.id,
            OutboxEvent::WalletAutoTopUpCapReached(event) => event.id,
            OutboxEvent::EntitlementUsageThresholdReached(event)
            | OutboxEvent::EntitlementLimitReached(event) => event.id,
            OutboxEvent::BudgetThresholdReached(event) | OutboxEvent::BudgetExceeded(event) => {
                event.id
            }
            OutboxEvent::EntitlementTooManyGroups(event) => event.id,
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
            OutboxEvent::InvoiceClosed(event) => event.tenant_id,
            OutboxEvent::InvoiceConsolidated(event) => event.tenant_id,
            OutboxEvent::InvoiceDeleted(event) => event.tenant_id,
            OutboxEvent::InvoiceAccountingPdfGenerated(event) => event.tenant_id,
            OutboxEvent::CreditNoteCreated(event) => event.tenant_id,
            OutboxEvent::CreditNoteFinalized(event) => event.tenant_id,
            OutboxEvent::CreditNoteVoided(event) => event.tenant_id,
            OutboxEvent::SubscriptionCreated(event) => event.tenant_id,
            OutboxEvent::SubscriptionUpdated(event) => event.tenant_id,
            OutboxEvent::SubscriptionCancelled(event) => event.tenant_id,
            OutboxEvent::SubscriptionEnded(event) => event.tenant_id,
            OutboxEvent::PaymentTransactionSaved(event) => event.tenant_id,
            OutboxEvent::RefundIssued(event) => event.tenant_id,
            OutboxEvent::RefundSettled(event) => event.tenant_id,
            OutboxEvent::RefundFailed(event) => event.tenant_id,
            OutboxEvent::PaymentReversed(event) => event.tenant_id,
            OutboxEvent::PaymentReinstated(event) => event.tenant_id,
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
            OutboxEvent::WalletFunded(event) => event.tenant_id,
            OutboxEvent::WalletDebited(event) => event.tenant_id,
            OutboxEvent::CreditGrantExpired(event) => event.tenant_id,
            OutboxEvent::WalletBalanceLow(event) => event.tenant_id,
            OutboxEvent::WalletCreditUnrecovered(event) => event.tenant_id,
            OutboxEvent::WalletCreditRecovered(event) => event.tenant_id,
            OutboxEvent::WalletAutoTopUpFailed(event) => event.tenant_id,
            OutboxEvent::WalletAutoTopUpSuspended(event) => event.tenant_id,
            OutboxEvent::WalletAutoTopUpConsentChanged(event) => event.tenant_id,
            OutboxEvent::WalletDepleted(event) => event.tenant_id,
            OutboxEvent::WalletRecovered(event) => event.tenant_id,
            OutboxEvent::WalletAutoTopUpCapReached(event) => event.tenant_id,
            OutboxEvent::EntitlementUsageThresholdReached(event)
            | OutboxEvent::EntitlementLimitReached(event) => event.tenant_id,
            OutboxEvent::BudgetThresholdReached(event) | OutboxEvent::BudgetExceeded(event) => {
                event.tenant_id
            }
            OutboxEvent::EntitlementTooManyGroups(event) => event.tenant_id,
        }
    }

    /// The customer whose credit the event can change; none for events that cannot (catalog,
    /// payment and quote events, whose effect on credit arrives with their invoice or subscription).
    pub fn customer_id(&self) -> Option<CustomerId> {
        match self {
            OutboxEvent::CustomerCreated(event) | OutboxEvent::CustomerUpdated(event) => {
                Some(event.customer_id)
            }
            OutboxEvent::InvoiceCreated(event)
            | OutboxEvent::InvoiceFinalized(event)
            | OutboxEvent::InvoicePaid(event)
            | OutboxEvent::InvoiceVoided(event)
            | OutboxEvent::InvoiceClosed(event)
            | OutboxEvent::InvoiceConsolidated(event)
            | OutboxEvent::InvoiceDeleted(event) => Some(event.customer_id),
            OutboxEvent::CreditNoteCreated(event)
            | OutboxEvent::CreditNoteFinalized(event)
            | OutboxEvent::CreditNoteVoided(event) => Some(event.customer_id),
            OutboxEvent::SubscriptionCreated(event)
            | OutboxEvent::SubscriptionUpdated(event)
            | OutboxEvent::SubscriptionCancelled(event)
            | OutboxEvent::SubscriptionEnded(event) => Some(event.customer_id),
            OutboxEvent::WalletFunded(event) => Some(event.customer_id),
            OutboxEvent::WalletDebited(event) => Some(event.customer_id),
            OutboxEvent::CreditGrantExpired(event) => Some(event.customer_id),
            OutboxEvent::WalletBalanceLow(event) => Some(event.customer_id),
            OutboxEvent::WalletCreditUnrecovered(event) => Some(event.customer_id),
            OutboxEvent::WalletCreditRecovered(event) => Some(event.customer_id),
            OutboxEvent::WalletAutoTopUpFailed(event)
            | OutboxEvent::WalletAutoTopUpSuspended(event) => Some(event.customer_id),
            OutboxEvent::WalletAutoTopUpConsentChanged(event) => Some(event.customer_id),
            OutboxEvent::WalletDepleted(event) => Some(event.customer_id),
            OutboxEvent::WalletRecovered(event) => Some(event.customer_id),
            OutboxEvent::WalletAutoTopUpCapReached(event) => Some(event.customer_id),
            OutboxEvent::BillableMetricCreated(_)
            | OutboxEvent::BillableMetricUpdated(_)
            | OutboxEvent::BillableMetricArchived(_)
            | OutboxEvent::InvoiceAccountingPdfGenerated(_)
            | OutboxEvent::PaymentTransactionSaved(_)
            | OutboxEvent::RefundIssued(_)
            | OutboxEvent::RefundSettled(_)
            | OutboxEvent::RefundFailed(_)
            | OutboxEvent::PaymentReversed(_)
            | OutboxEvent::PaymentReinstated(_)
            | OutboxEvent::QuoteAccepted(_)
            | OutboxEvent::QuoteConverted(_)
            | OutboxEvent::PlanCreated(_)
            | OutboxEvent::PlanPublished(_)
            | OutboxEvent::PlanArchived(_)
            | OutboxEvent::ProductCreated(_)
            | OutboxEvent::ProductUpdated(_)
            | OutboxEvent::ProductArchived(_)
            | OutboxEvent::CouponCreated(_)
            | OutboxEvent::CouponUpdated(_)
            | OutboxEvent::CouponArchived(_)
            | OutboxEvent::AddOnCreated(_)
            | OutboxEvent::AddOnUpdated(_)
            | OutboxEvent::AddOnArchived(_)
            // Alerts follow the recompute that reports them; none of them changes credit.
            | OutboxEvent::EntitlementUsageThresholdReached(_)
            | OutboxEvent::EntitlementLimitReached(_)
            | OutboxEvent::BudgetThresholdReached(_)
            | OutboxEvent::BudgetExceeded(_)
            | OutboxEvent::EntitlementTooManyGroups(_) => None,
        }
    }

    /// The subscription the event is about, for correlating its handling in logs and traces.
    pub fn subscription_id(&self) -> Option<SubscriptionId> {
        match self {
            OutboxEvent::InvoiceCreated(event)
            | OutboxEvent::InvoiceFinalized(event)
            | OutboxEvent::InvoicePaid(event)
            | OutboxEvent::InvoiceVoided(event)
            | OutboxEvent::InvoiceClosed(event)
            | OutboxEvent::InvoiceConsolidated(event)
            | OutboxEvent::InvoiceDeleted(event) => event.subscription_id,
            OutboxEvent::SubscriptionCreated(event)
            | OutboxEvent::SubscriptionUpdated(event)
            | OutboxEvent::SubscriptionCancelled(event)
            | OutboxEvent::SubscriptionEnded(event) => Some(event.subscription_id),
            _ => None,
        }
    }

    /// The invoice the event is about, for correlating its handling in logs and traces.
    pub fn invoice_id(&self) -> Option<InvoiceId> {
        match self {
            OutboxEvent::InvoiceCreated(event)
            | OutboxEvent::InvoiceFinalized(event)
            | OutboxEvent::InvoicePaid(event)
            | OutboxEvent::InvoiceVoided(event)
            | OutboxEvent::InvoiceClosed(event)
            | OutboxEvent::InvoiceConsolidated(event)
            | OutboxEvent::InvoiceDeleted(event) => Some(event.invoice_id),
            OutboxEvent::InvoiceAccountingPdfGenerated(event) => Some(event.invoice_id),
            OutboxEvent::PaymentTransactionSaved(event)
            | OutboxEvent::RefundIssued(event)
            | OutboxEvent::RefundSettled(event)
            | OutboxEvent::RefundFailed(event)
            | OutboxEvent::PaymentReversed(event)
            | OutboxEvent::PaymentReinstated(event) => event.invoice_id,
            _ => None,
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
            OutboxEvent::InvoiceClosed(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoiceConsolidated(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoiceDeleted(event) => event.invoice_id.as_uuid(),
            OutboxEvent::InvoiceAccountingPdfGenerated(event) => event.invoice_id.as_uuid(),
            OutboxEvent::CreditNoteCreated(event) => event.credit_note_id.as_uuid(),
            OutboxEvent::CreditNoteFinalized(event) => event.credit_note_id.as_uuid(),
            OutboxEvent::CreditNoteVoided(event) => event.credit_note_id.as_uuid(),
            OutboxEvent::SubscriptionCreated(event) => event.subscription_id.as_uuid(),
            OutboxEvent::SubscriptionUpdated(event) => event.subscription_id.as_uuid(),
            OutboxEvent::SubscriptionCancelled(event) => event.subscription_id.as_uuid(),
            OutboxEvent::SubscriptionEnded(event) => event.subscription_id.as_uuid(),
            OutboxEvent::PaymentTransactionSaved(event) => event.payment_transaction_id.as_uuid(),
            OutboxEvent::RefundIssued(event) => event.payment_transaction_id.as_uuid(),
            OutboxEvent::RefundSettled(event) => event.payment_transaction_id.as_uuid(),
            OutboxEvent::RefundFailed(event) => event.payment_transaction_id.as_uuid(),
            OutboxEvent::PaymentReversed(event) => event.payment_transaction_id.as_uuid(),
            OutboxEvent::PaymentReinstated(event) => event.payment_transaction_id.as_uuid(),
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
            OutboxEvent::WalletFunded(event) => event.wallet_id.as_uuid(),
            OutboxEvent::WalletDebited(event) => event.wallet_id.as_uuid(),
            OutboxEvent::CreditGrantExpired(event) => event.credit_grant_id.as_uuid(),
            OutboxEvent::WalletBalanceLow(event) => event.wallet_id.as_uuid(),
            OutboxEvent::WalletCreditUnrecovered(event) => event.wallet_id.as_uuid(),
            OutboxEvent::WalletCreditRecovered(event) => event.wallet_id.as_uuid(),
            OutboxEvent::WalletAutoTopUpFailed(event) => event.wallet_id.as_uuid(),
            OutboxEvent::WalletAutoTopUpSuspended(event) => event.wallet_id.as_uuid(),
            OutboxEvent::WalletAutoTopUpConsentChanged(event) => event.wallet_id.as_uuid(),
            OutboxEvent::WalletDepleted(event) => event.wallet_id.as_uuid(),
            OutboxEvent::WalletRecovered(event) => event.wallet_id.as_uuid(),
            OutboxEvent::WalletAutoTopUpCapReached(event) => event.wallet_id.as_uuid(),
            OutboxEvent::EntitlementUsageThresholdReached(event)
            | OutboxEvent::EntitlementLimitReached(event) => event.feature_id.as_uuid(),
            OutboxEvent::BudgetThresholdReached(event) | OutboxEvent::BudgetExceeded(event) => {
                event.budget_id.as_uuid()
            }
            OutboxEvent::EntitlementTooManyGroups(event) => event
                .feature_id
                .map_or_else(|| event.customer_id.as_uuid(), |id| id.as_uuid()),
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
            OutboxEvent::InvoiceClosed(_) => "Invoice".to_string(),
            OutboxEvent::InvoiceConsolidated(_) => "Invoice".to_string(),
            OutboxEvent::InvoiceDeleted(_) => "Invoice".to_string(),
            OutboxEvent::InvoiceAccountingPdfGenerated(_) => "Invoice".to_string(),
            OutboxEvent::CreditNoteCreated(_) => "CreditNote".to_string(),
            OutboxEvent::CreditNoteFinalized(_) => "CreditNote".to_string(),
            OutboxEvent::CreditNoteVoided(_) => "CreditNote".to_string(),
            OutboxEvent::SubscriptionCreated(_) => "Subscription".to_string(),
            OutboxEvent::SubscriptionUpdated(_) => "Subscription".to_string(),
            OutboxEvent::SubscriptionCancelled(_) => "Subscription".to_string(),
            OutboxEvent::SubscriptionEnded(_) => "Subscription".to_string(),
            OutboxEvent::PaymentTransactionSaved(_) => "PaymentTransaction".to_string(),
            OutboxEvent::RefundIssued(_) => "PaymentTransaction".to_string(),
            OutboxEvent::RefundSettled(_) => "PaymentTransaction".to_string(),
            OutboxEvent::RefundFailed(_) => "PaymentTransaction".to_string(),
            OutboxEvent::PaymentReversed(_) => "PaymentTransaction".to_string(),
            OutboxEvent::PaymentReinstated(_) => "PaymentTransaction".to_string(),
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
            OutboxEvent::WalletFunded(_) => "CreditWallet".to_string(),
            OutboxEvent::WalletDebited(_) => "CreditWallet".to_string(),
            OutboxEvent::CreditGrantExpired(_) => "CreditGrant".to_string(),
            OutboxEvent::WalletBalanceLow(_) => "CreditWallet".to_string(),
            OutboxEvent::WalletCreditUnrecovered(_) => "CreditWallet".to_string(),
            OutboxEvent::WalletCreditRecovered(_) => "CreditWallet".to_string(),
            OutboxEvent::WalletAutoTopUpFailed(_) => "CreditWallet".to_string(),
            OutboxEvent::WalletAutoTopUpSuspended(_) => "CreditWallet".to_string(),
            OutboxEvent::WalletAutoTopUpConsentChanged(_) => "CreditWallet".to_string(),
            OutboxEvent::WalletDepleted(_) => "CreditWallet".to_string(),
            OutboxEvent::WalletRecovered(_) => "CreditWallet".to_string(),
            OutboxEvent::WalletAutoTopUpCapReached(_) => "CreditWallet".to_string(),
            OutboxEvent::EntitlementUsageThresholdReached(_)
            | OutboxEvent::EntitlementLimitReached(_) => "Feature".to_string(),
            OutboxEvent::BudgetThresholdReached(_) | OutboxEvent::BudgetExceeded(_) => {
                "Budget".to_string()
            }
            OutboxEvent::EntitlementTooManyGroups(_) => "Feature".to_string(),
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
            OutboxEvent::InvoiceClosed(_) => EventType::InvoiceClosed,
            OutboxEvent::InvoiceConsolidated(_) => EventType::InvoiceConsolidated,
            OutboxEvent::InvoiceDeleted(_) => EventType::InvoiceDeleted,
            OutboxEvent::InvoiceAccountingPdfGenerated(_) => {
                EventType::InvoiceAccountingPdfGenerated
            }
            OutboxEvent::CreditNoteCreated(_) => EventType::CreditNoteCreated,
            OutboxEvent::CreditNoteFinalized(_) => EventType::CreditNoteFinalized,
            OutboxEvent::CreditNoteVoided(_) => EventType::CreditNoteVoided,
            OutboxEvent::SubscriptionCreated(_) => EventType::SubscriptionCreated,
            OutboxEvent::SubscriptionUpdated(_) => EventType::SubscriptionUpdated,
            OutboxEvent::SubscriptionCancelled(_) => EventType::SubscriptionCancelled,
            OutboxEvent::SubscriptionEnded(_) => EventType::SubscriptionEnded,
            OutboxEvent::PaymentTransactionSaved(_) => EventType::PaymentTransactionReceived,
            OutboxEvent::RefundIssued(_) => EventType::RefundIssued,
            OutboxEvent::RefundSettled(_) => EventType::RefundSettled,
            OutboxEvent::RefundFailed(_) => EventType::RefundFailed,
            OutboxEvent::PaymentReversed(_) => EventType::PaymentReversed,
            OutboxEvent::PaymentReinstated(_) => EventType::PaymentReinstated,
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
            OutboxEvent::WalletFunded(_) => EventType::WalletFunded,
            OutboxEvent::WalletDebited(_) => EventType::WalletDebited,
            OutboxEvent::CreditGrantExpired(_) => EventType::CreditGrantExpired,
            OutboxEvent::WalletBalanceLow(_) => EventType::WalletBalanceLow,
            OutboxEvent::WalletCreditUnrecovered(_) => EventType::WalletCreditUnrecovered,
            OutboxEvent::WalletCreditRecovered(_) => EventType::WalletCreditRecovered,
            OutboxEvent::WalletAutoTopUpFailed(_) => EventType::WalletAutoTopUpFailed,
            OutboxEvent::WalletAutoTopUpSuspended(_) => EventType::WalletAutoTopUpSuspended,
            OutboxEvent::WalletAutoTopUpConsentChanged(_) => {
                EventType::WalletAutoTopUpConsentChanged
            }
            OutboxEvent::WalletDepleted(_) => EventType::WalletDepleted,
            OutboxEvent::WalletRecovered(_) => EventType::WalletRecovered,
            OutboxEvent::WalletAutoTopUpCapReached(_) => EventType::WalletAutoTopUpCapReached,
            OutboxEvent::EntitlementUsageThresholdReached(_) => {
                EventType::EntitlementUsageThresholdReached
            }
            OutboxEvent::EntitlementLimitReached(_) => EventType::EntitlementLimitReached,
            OutboxEvent::BudgetThresholdReached(_) => EventType::BudgetThresholdReached,
            OutboxEvent::BudgetExceeded(_) => EventType::BudgetExceeded,
            OutboxEvent::EntitlementTooManyGroups(_) => EventType::EntitlementTooManyGroups,
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

    pub fn invoice_consolidated(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoiceConsolidated(Box::new(event))
    }

    pub fn invoice_voided(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoiceVoided(Box::new(event))
    }

    pub fn invoice_closed(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoiceClosed(Box::new(event))
    }

    pub fn invoice_deleted(event: InvoiceEvent) -> OutboxEvent {
        OutboxEvent::InvoiceDeleted(Box::new(event))
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

    pub fn subscription_updated(event: SubscriptionEvent) -> OutboxEvent {
        OutboxEvent::SubscriptionUpdated(Box::new(event))
    }

    pub fn subscription_cancelled(event: SubscriptionEvent) -> OutboxEvent {
        OutboxEvent::SubscriptionCancelled(Box::new(event))
    }

    pub fn subscription_ended(event: SubscriptionEvent) -> OutboxEvent {
        OutboxEvent::SubscriptionEnded(Box::new(event))
    }

    pub fn payment_transaction_saved(event: PaymentTransactionEvent) -> OutboxEvent {
        OutboxEvent::PaymentTransactionSaved(Box::new(event))
    }

    pub fn refund_issued(event: PaymentTransactionEvent) -> OutboxEvent {
        OutboxEvent::RefundIssued(Box::new(event))
    }

    pub fn refund_settled(event: PaymentTransactionEvent) -> OutboxEvent {
        OutboxEvent::RefundSettled(Box::new(event))
    }

    pub fn refund_failed(event: PaymentTransactionEvent) -> OutboxEvent {
        OutboxEvent::RefundFailed(Box::new(event))
    }

    pub fn payment_reversed(event: PaymentTransactionEvent) -> OutboxEvent {
        OutboxEvent::PaymentReversed(Box::new(event))
    }

    pub fn payment_reinstated(event: PaymentTransactionEvent) -> OutboxEvent {
        OutboxEvent::PaymentReinstated(Box::new(event))
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

    pub fn wallet_funded(event: WalletFundedEvent) -> OutboxEvent {
        OutboxEvent::WalletFunded(Box::new(event))
    }

    pub fn wallet_debited(event: WalletDebitedEvent) -> OutboxEvent {
        OutboxEvent::WalletDebited(Box::new(event))
    }

    pub fn credit_grant_expired(event: CreditGrantExpiredEvent) -> OutboxEvent {
        OutboxEvent::CreditGrantExpired(Box::new(event))
    }

    /// The limit reached, when the crossed level is the limit itself, else a threshold.
    pub fn entitlement_alert(event: EntitlementAlertEvent) -> OutboxEvent {
        if is_limit_pct(event.threshold_pct) {
            OutboxEvent::EntitlementLimitReached(Box::new(event))
        } else {
            OutboxEvent::EntitlementUsageThresholdReached(Box::new(event))
        }
    }

    pub fn budget_alert(event: BudgetAlertEvent) -> OutboxEvent {
        if is_limit_pct(event.threshold_pct) {
            OutboxEvent::BudgetExceeded(Box::new(event))
        } else {
            OutboxEvent::BudgetThresholdReached(Box::new(event))
        }
    }

    pub fn entitlement_too_many_groups(event: TooManyGroupsEvent) -> OutboxEvent {
        OutboxEvent::EntitlementTooManyGroups(Box::new(event))
    }

    pub fn wallet_balance_low(event: WalletBalanceLowEvent) -> OutboxEvent {
        OutboxEvent::WalletBalanceLow(Box::new(event))
    }

    pub fn wallet_credit_unrecovered(event: WalletCreditUnrecoveredEvent) -> OutboxEvent {
        OutboxEvent::WalletCreditUnrecovered(Box::new(event))
    }

    pub fn wallet_credit_recovered(event: WalletCreditRecoveredEvent) -> OutboxEvent {
        OutboxEvent::WalletCreditRecovered(Box::new(event))
    }

    pub fn wallet_auto_top_up_failed(event: WalletAutoTopUpEvent) -> OutboxEvent {
        OutboxEvent::WalletAutoTopUpFailed(Box::new(event))
    }

    pub fn wallet_auto_top_up_suspended(event: WalletAutoTopUpEvent) -> OutboxEvent {
        OutboxEvent::WalletAutoTopUpSuspended(Box::new(event))
    }

    pub fn wallet_auto_top_up_consent_changed(
        event: WalletAutoTopUpConsentChangedEvent,
    ) -> OutboxEvent {
        OutboxEvent::WalletAutoTopUpConsentChanged(Box::new(event))
    }

    pub fn wallet_depleted(event: WalletDepletedEvent) -> OutboxEvent {
        OutboxEvent::WalletDepleted(Box::new(event))
    }

    pub fn wallet_recovered(event: WalletRecoveredEvent) -> OutboxEvent {
        OutboxEvent::WalletRecovered(Box::new(event))
    }

    pub fn wallet_auto_top_up_cap_reached(event: WalletAutoTopUpCapReachedEvent) -> OutboxEvent {
        OutboxEvent::WalletAutoTopUpCapReached(Box::new(event))
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
    #[serde(default)]
    pub custom_properties: serde_json::Value,
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
#[ghosts(archived_at: None, updated_at: None, filters: None, usage_scope: {crate::domain::UsageScope::default()})]
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
    pub unit_singular: Option<String>,
    pub unit_plural: Option<String>,
    pub created_at: NaiveDateTime,
    pub product_family_id: ProductFamilyId,
    pub product_id: Option<ProductId>,
}

/// Discriminates what changed on a `subscription.updated` webhook so integrators can
/// route without diffing. Terminal cancellation/abort is a `subscription.cancelled`
/// event instead; `Paused`/`Completed` ride on `updated` with the matching `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionUpdateType {
    Activated,
    TrialEnded,
    BillingConfigurationUpdated,
    PlanChanged,
    Amended,
    UnitsChanged,
    Paused,
    CancellationScheduled,
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
    pub net_terms: u32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub mrr_cents: u64,
    pub period: BillingPeriodEnum,
    pub status: SubscriptionStatusEnum,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub purchase_order: Option<String>,
    /// Set on `subscription.updated`; identifies which mutation fired the event.
    #[ghost({None})]
    pub change_type: Option<SubscriptionUpdateType>,
    /// Set on `subscription.cancelled` when a reason was supplied at cancellation.
    #[ghost({None})]
    pub cancellation_reason: Option<String>,
    #[serde(default)]
    pub custom_properties: serde_json::Value,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_ref(Invoice)]
pub struct InvoiceEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[map(@.id)]
    pub invoice_id: InvoiceId,
    /// `None` while the invoice is a draft — the number is assigned at finalization.
    /// A Closed invoice (empty recurring period) is never numbered either, so it must
    /// also report `None` rather than the "draft" placeholder its row still carries.
    #[serde(default)]
    #[map(match @.status {
        InvoiceStatusEnum::Draft | InvoiceStatusEnum::Closed => None,
        _ => Some(@.invoice_number.clone()),
    })]
    pub invoice_number: Option<String>,
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
    pub parent_invoice_id: Option<InvoiceId>,
    /// Set on a child draft merged into a consolidated invoice; references that parent.
    pub consolidated_into_invoice_id: Option<InvoiceId>,
    #[serde(default)]
    #[map(@.custom_properties.clone())]
    pub custom_properties: serde_json::Value,
    /// Set on `invoice.paid` when credit notes, not a payment, settled it (see
    /// `Invoice::settled_by_credit_notes`).
    #[serde(default)]
    #[ghost({false})]
    pub settled_by_credit_notes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoicePdfGeneratedEvent {
    pub id: EventId,
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub pdf_id: StoredDocumentId,
    /// The structured e-invoice stored beside the PDF, when one was produced.
    #[serde(default)]
    pub xml_id: Option<StoredDocumentId>,
    /// `None` when the entity had not opted in; `Failed` carries its reasons in the report.
    #[serde(default)]
    pub einvoicing_status: Option<EInvoicingStatus>,
    #[serde(default)]
    pub einvoicing_report: Option<EInvoicingReport>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_ref(CreditNote)]
pub struct CreditNoteEvent {
    #[map(EventId::new())]
    pub id: EventId,
    #[map(@.id)]
    pub credit_note_id: CreditNoteId,
    /// `None` while the credit note is a draft — the number is assigned at finalization.
    #[serde(default)]
    #[map(match @.status {
        CreditNoteStatus::Draft => None,
        _ => Some(@.credit_note_number.clone()),
    })]
    pub credit_note_number: Option<String>,
    #[map(@.status.clone())]
    pub status: CreditNoteStatus,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub invoice_id: InvoiceId,
    /// Number of the invoice being credited, snapshotted when the credit note was created.
    /// `None` only on messages queued before this field existed.
    #[serde(default)]
    #[map(Some(@.invoice_number.clone()))]
    pub invoice_number: Option<String>,
    pub subscription_id: Option<SubscriptionId>,
    pub plan_version_id: Option<PlanVersionId>,
    #[map(@.currency.clone())]
    pub currency: String,
    pub subtotal: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub refunded_amount_cents: i64,
    pub credited_amount_cents: i64,
    #[serde(default)]
    #[map(@.line_items.clone())]
    pub line_items: Vec<LineItem>,
    #[serde(default)]
    #[map(@.tax_breakdown.clone())]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    #[serde(default)]
    #[map(@.reason.clone())]
    pub reason: Option<String>,
    #[serde(default)]
    #[map(@.memo.clone())]
    pub memo: Option<String>,
    #[map(@.created_at.clone())]
    pub created_at: NaiveDateTime,
    #[map(@.conn_meta.clone())]
    pub conn_meta: Option<ConnectionMeta>,
    #[serde(default)]
    #[map(@.custom_properties.clone())]
    pub custom_properties: serde_json::Value,
    pub finalized_at: Option<NaiveDateTime>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[map_owned(PaymentTransaction)]
// next_action is transient and the pending hosted-intent marker is internal
// bookkeeping — neither is carried on the persisted event.
#[ghosts(next_action: {None}, pending_provider_intent_id: {None}, pending_connection_id: {None}, reversal_reason: {None})]
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
    // Default so events serialized before this field existed still decode
    // (in-flight queue messages across a deploy).
    #[serde(default)]
    pub amount_refunded: i64,
    #[serde(default)]
    pub amount_reversed: i64,
    #[serde(default)]
    pub reversal_kind: Option<ReversalKind>,
    #[serde(default)]
    pub amount_disputed: i64,
    #[serde(default)]
    pub amount_reversed_excess: i64,
    #[serde(default)]
    pub parent_transaction_id: Option<PaymentTransactionId>,
    #[serde(default)]
    pub credit_note_id: Option<CreditNoteId>,
    #[serde(default)]
    pub refund_mode: Option<RefundMode>,
    #[serde(default)]
    pub decline_kind: Option<DeclineKind>,
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

impl OutboxEvent {
    /// Topic routing key used when publishing this event via pgmq `send_topic`.
    pub fn routing_key(&self) -> &'static str {
        self.event_type().routing_key()
    }
}

impl OutboxEvent {
    /// Documents which events come from a writer of live-credit inputs. The bumps themselves are
    /// explicit calls at each writer (`credit_inputs::bump_customer`, `transaction_bumping_tenant`).
    pub fn affects_credit_inputs(&self) -> bool {
        self.event_type().affects_credit_inputs()
    }
}

impl EventType {
    pub fn affects_credit_inputs(&self) -> bool {
        match self {
            EventType::CustomerUpdated
            | EventType::InvoiceCreated
            | EventType::InvoiceFinalized
            | EventType::InvoiceVoided
            | EventType::InvoiceClosed
            | EventType::InvoiceConsolidated
            | EventType::InvoiceDeleted
            | EventType::CreditNoteFinalized
            | EventType::CreditNoteVoided
            | EventType::SubscriptionCreated
            | EventType::SubscriptionUpdated
            | EventType::SubscriptionCancelled
            | EventType::SubscriptionEnded
            | EventType::PlanPublished
            | EventType::ProductUpdated
            | EventType::BillableMetricUpdated
            | EventType::BillableMetricArchived
            | EventType::CouponUpdated
            | EventType::AddOnUpdated
            | EventType::WalletAutoTopUpConsentChanged => true,
            EventType::CustomerCreated
            | EventType::BillableMetricCreated
            | EventType::InvoicePaid
            | EventType::InvoiceAccountingPdfGenerated
            | EventType::CreditNoteCreated
            | EventType::PaymentTransactionReceived
            | EventType::RefundIssued
            | EventType::RefundSettled
            | EventType::RefundFailed
            | EventType::PaymentReversed
            | EventType::PaymentReinstated
            | EventType::QuoteAccepted
            | EventType::QuoteConverted
            | EventType::PlanCreated
            | EventType::PlanArchived
            | EventType::ProductCreated
            | EventType::ProductArchived
            | EventType::CouponCreated
            | EventType::CouponArchived
            | EventType::AddOnCreated
            | EventType::AddOnArchived
            | EventType::WalletFunded
            | EventType::WalletDebited
            | EventType::CreditGrantExpired
            | EventType::WalletBalanceLow
            | EventType::WalletCreditUnrecovered
            | EventType::WalletCreditRecovered
            | EventType::WalletAutoTopUpFailed
            | EventType::WalletAutoTopUpSuspended
            | EventType::WalletDepleted
            | EventType::WalletRecovered
            | EventType::WalletAutoTopUpCapReached
            | EventType::EntitlementUsageThresholdReached
            | EventType::EntitlementLimitReached
            | EventType::BudgetThresholdReached
            | EventType::BudgetExceeded
            | EventType::EntitlementTooManyGroups => false,
        }
    }
}

impl EventType {
    /// Topic routing key for pgmq `send_topic`, `entity.event`. Exhaustive so a new
    /// event variant fails to compile until it declares one. Keys are a stable public
    /// contract (consumer bindings match on them) — rename with care.
    pub fn routing_key(&self) -> &'static str {
        match self {
            EventType::CustomerCreated => "customer.created",
            EventType::CustomerUpdated => "customer.updated",
            EventType::SubscriptionCreated => "subscription.created",
            EventType::SubscriptionUpdated => "subscription.updated",
            EventType::SubscriptionCancelled => "subscription.cancelled",
            EventType::SubscriptionEnded => "subscription.ended",
            EventType::InvoiceCreated => "invoice.created",
            EventType::InvoiceFinalized => "invoice.finalized",
            EventType::InvoicePaid => "invoice.paid",
            EventType::InvoiceVoided => "invoice.voided",
            EventType::InvoiceClosed => "invoice.closed",
            EventType::InvoiceConsolidated => "invoice.consolidated",
            EventType::InvoiceDeleted => "invoice.deleted",
            EventType::InvoiceAccountingPdfGenerated => "invoice.accounting_pdf_generated",
            EventType::CreditNoteCreated => "credit_note.created",
            EventType::CreditNoteFinalized => "credit_note.finalized",
            EventType::CreditNoteVoided => "credit_note.voided",
            EventType::PaymentTransactionReceived => "payment.transaction_received",
            EventType::RefundIssued => "refund.issued",
            EventType::RefundSettled => "refund.settled",
            EventType::RefundFailed => "refund.failed",
            EventType::PaymentReversed => "payment.reversed",
            EventType::PaymentReinstated => "payment.reinstated",
            EventType::QuoteAccepted => "quote.accepted",
            EventType::QuoteConverted => "quote.converted",
            EventType::PlanCreated => "plan.created",
            EventType::PlanPublished => "plan.published",
            EventType::PlanArchived => "plan.archived",
            EventType::ProductCreated => "product.created",
            EventType::ProductUpdated => "product.updated",
            EventType::ProductArchived => "product.archived",
            EventType::BillableMetricCreated => "billable_metric.created",
            EventType::BillableMetricUpdated => "billable_metric.updated",
            EventType::BillableMetricArchived => "billable_metric.archived",
            EventType::CouponCreated => "coupon.created",
            EventType::CouponUpdated => "coupon.updated",
            EventType::CouponArchived => "coupon.archived",
            EventType::AddOnCreated => "add_on.created",
            EventType::AddOnUpdated => "add_on.updated",
            EventType::AddOnArchived => "add_on.archived",
            EventType::WalletFunded => WalletFundedEvent::ROUTING_KEY,
            EventType::WalletDebited => WalletDebitedEvent::ROUTING_KEY,
            EventType::CreditGrantExpired => CreditGrantExpiredEvent::ROUTING_KEY,
            EventType::WalletBalanceLow => WalletBalanceLowEvent::ROUTING_KEY,
            EventType::WalletCreditUnrecovered => WalletCreditUnrecoveredEvent::ROUTING_KEY,
            EventType::WalletCreditRecovered => WalletCreditRecoveredEvent::ROUTING_KEY,
            EventType::WalletAutoTopUpFailed => WalletAutoTopUpEvent::FAILED_ROUTING_KEY,
            EventType::WalletAutoTopUpSuspended => WalletAutoTopUpEvent::SUSPENDED_ROUTING_KEY,
            EventType::WalletAutoTopUpConsentChanged => {
                WalletAutoTopUpConsentChangedEvent::ROUTING_KEY
            }
            EventType::WalletDepleted => WalletDepletedEvent::ROUTING_KEY,
            EventType::WalletRecovered => WalletRecoveredEvent::ROUTING_KEY,
            EventType::WalletAutoTopUpCapReached => WalletAutoTopUpCapReachedEvent::ROUTING_KEY,
            EventType::EntitlementUsageThresholdReached => {
                EntitlementAlertEvent::THRESHOLD_ROUTING_KEY
            }
            EventType::EntitlementLimitReached => EntitlementAlertEvent::LIMIT_ROUTING_KEY,
            EventType::BudgetThresholdReached => BudgetAlertEvent::THRESHOLD_ROUTING_KEY,
            EventType::BudgetExceeded => BudgetAlertEvent::EXCEEDED_ROUTING_KEY,
            EventType::EntitlementTooManyGroups => TooManyGroupsEvent::ENTITLEMENT_ROUTING_KEY,
        }
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

#[cfg(test)]
mod credit_inputs_tests {
    use super::EventType;
    use strum::IntoEnumIterator;

    // Every event, with whether its writer changes a live-credit input and why. A new event fails
    // here until it is decided.
    const DECISIONS: &[(EventType, bool, &str)] = &[
        (
            EventType::CustomerCreated,
            false,
            "a new customer has no cached inputs; its first input writer bumps",
        ),
        (
            EventType::CustomerUpdated,
            true,
            "billing, tax and currency data of the customer",
        ),
        (
            EventType::BillableMetricCreated,
            false,
            "unreferenced until a price uses it",
        ),
        (EventType::InvoiceCreated, true, "the draft set (rev 6 4.2)"),
        (
            EventType::InvoiceFinalized,
            true,
            "invoiced spend and the draft set",
        ),
        (
            EventType::InvoicePaid,
            false,
            "payment status prices nothing; a payment's side effects (activation, funding) bump or record ledger at their own writer",
        ),
        (EventType::InvoiceVoided, true, "invoiced spend"),
        (EventType::InvoiceClosed, true, "the draft set (rev 6 4.2)"),
        (EventType::InvoiceConsolidated, true, "the draft set"),
        (EventType::InvoiceDeleted, true, "the draft set"),
        (
            EventType::InvoiceAccountingPdfGenerated,
            false,
            "a document render",
        ),
        (
            EventType::CreditNoteCreated,
            false,
            "a draft note changes no spend",
        ),
        (EventType::CreditNoteFinalized, true, "invoiced spend"),
        (EventType::CreditNoteVoided, true, "invoiced spend"),
        (
            EventType::SubscriptionCreated,
            true,
            "plan, phases and components",
        ),
        (
            EventType::SubscriptionUpdated,
            true,
            "plan, phases and components",
        ),
        (
            EventType::SubscriptionCancelled,
            true,
            "phases and period end",
        ),
        (EventType::SubscriptionEnded, true, "phases and period end"),
        (
            EventType::PaymentTransactionReceived,
            false,
            "money movement: ledger effects write the ledger feed row",
        ),
        (EventType::RefundIssued, false, "money movement"),
        (EventType::RefundSettled, false, "money movement"),
        (EventType::RefundFailed, false, "money movement"),
        (EventType::PaymentReversed, false, "money movement"),
        (EventType::PaymentReinstated, false, "money movement"),
        (
            EventType::QuoteAccepted,
            false,
            "no subscription exists yet",
        ),
        (
            EventType::QuoteConverted,
            false,
            "the conversion's subscription insert bumps the customer, in the same transaction",
        ),
        (EventType::PlanCreated, false, "no subscriber yet"),
        (
            EventType::PlanPublished,
            true,
            "the active version a trialing subscription's effective plan reads",
        ),
        (
            EventType::PlanArchived,
            false,
            "existing subscriptions keep their version",
        ),
        (
            EventType::ProductCreated,
            false,
            "unreferenced until a component uses it",
        ),
        (
            EventType::ProductUpdated,
            true,
            "fee structure and tax category of live components",
        ),
        (
            EventType::ProductArchived,
            false,
            "existing components keep pricing",
        ),
        (
            EventType::BillableMetricUpdated,
            true,
            "segmentation, filters and unit conversion",
        ),
        (
            EventType::BillableMetricArchived,
            true,
            "metric definition change (rev 6 14)",
        ),
        (
            EventType::CouponCreated,
            false,
            "unreferenced until applied",
        ),
        (
            EventType::CouponUpdated,
            true,
            "the discount of applied coupons",
        ),
        (
            EventType::CouponArchived,
            false,
            "only checked at redemption",
        ),
        (
            EventType::AddOnCreated,
            false,
            "unreferenced until attached",
        ),
        (
            EventType::AddOnUpdated,
            true,
            "max instances read by entitlement resolution",
        ),
        (
            EventType::AddOnArchived,
            false,
            "attached add-ons keep their snapshot",
        ),
        (EventType::WalletFunded, false, "ledger"),
        (EventType::WalletDebited, false, "ledger"),
        (EventType::CreditGrantExpired, false, "ledger"),
        (EventType::WalletBalanceLow, false, "evaluator reaction"),
        (EventType::WalletCreditUnrecovered, false, "ledger"),
        (EventType::WalletCreditRecovered, false, "ledger"),
        (
            EventType::WalletAutoTopUpFailed,
            false,
            "top-up runtime feed row",
        ),
        (
            EventType::WalletAutoTopUpSuspended,
            false,
            "top-up runtime feed row",
        ),
        (
            EventType::WalletAutoTopUpConsentChanged,
            true,
            "auto top-up configuration",
        ),
        (EventType::WalletDepleted, false, "evaluator reaction"),
        (EventType::WalletRecovered, false, "evaluator reaction"),
        (
            EventType::WalletAutoTopUpCapReached,
            false,
            "evaluator reaction",
        ),
        (
            EventType::EntitlementUsageThresholdReached,
            false,
            "evaluator reaction",
        ),
        (
            EventType::EntitlementLimitReached,
            false,
            "evaluator reaction",
        ),
        (
            EventType::BudgetThresholdReached,
            false,
            "evaluator reaction",
        ),
        (EventType::BudgetExceeded, false, "evaluator reaction"),
        (
            EventType::EntitlementTooManyGroups,
            false,
            "evaluator reaction",
        ),
    ];

    #[test]
    fn every_event_is_classified() {
        for event in EventType::iter() {
            let decision = DECISIONS.iter().find(|(e, _, _)| *e == event);
            let Some((_, affects, reason)) = decision else {
                panic!("{event:?} has no credit input decision");
            };
            assert_eq!(
                event.affects_credit_inputs(),
                *affects,
                "{event:?}: {reason}"
            );
        }
        assert_eq!(DECISIONS.len(), EventType::iter().count());
    }
}
