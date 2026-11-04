use crate::coupons::model::CouponDiscount;
use crate::invoices::mapping::{
    map_decline_kind_to_rest, map_line_item_to_rest, map_payment_status_enum_to_rest,
    map_payment_type_to_rest, map_refund_mode_to_rest, map_reversal_kind_to_rest,
    map_tax_breakdown_to_rest,
};
use crate::invoices::model::{
    DeclineKind, EInvoicingStatus, InvoiceLineItem, InvoiceStatus, PaymentStatusEnum,
    PaymentTypeEnum, RefundMode, ReversalKind, TaxBreakdownItem,
};
use crate::metrics::model::{
    BillingMetricAggregateEnum, MetricSegmentationMatrix, UnitConversionRoundingEnum,
};
use crate::plans::model::{PlanStatusEnum as RestPlanStatusEnum, PlanTypeEnum as RestPlanTypeEnum};
use crate::products::model::ProductFeeTypeEnum;
use crate::subscriptions::model::SubscriptionStatusEnum;
use chrono::NaiveDate;
use common_domain::ids::{AddOnId, BaseId, CouponId, PriceId, UserId};
use common_domain::ids::{
    BillableMetricId, BudgetId, CreditGrantId, CreditNoteId, CustomerId, EventId, FeatureId,
    InvoiceId, LedgerEntryId, PaymentTransactionId, PlanId, ProductFamilyId, ProductId, QuoteId,
    SubscriptionId, string_serde, string_serde_opt, string_serde_vec,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::BillingPeriodEnum;
use meteroid_store::domain::outbox_event::{
    AddOnEvent, BillableMetricEvent, BudgetAlertEvent, CouponEvent, CreditGrantExpiredEvent,
    CreditNoteEvent, CustomerEvent, EntitlementAlertEvent, InvoiceEvent, InvoicePdfGeneratedEvent,
    PaymentTransactionEvent, PlanEvent, ProductEvent, QuoteAcceptedEvent, QuoteConvertedEvent,
    SubscriptionEvent, TooManyGroupsEvent, WalletAutoTopUpCapReachedEvent,
    WalletAutoTopUpConsentChangedEvent, WalletAutoTopUpEvent, WalletBalanceLowEvent,
    WalletCreditRecoveredEvent, WalletCreditUnrecoveredEvent, WalletDebitedEvent,
    WalletDepletedEvent, WalletFundedEvent, WalletRecoveredEvent,
};
use meteroid_store::domain::wallet_events::GrantLapseReason;
use o2o::o2o;
use serde::Serialize;
use serde_with::skip_serializing_none;
use std::str::FromStr;
use strum::{Display, EnumIter, EnumString};

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
    /// User-defined custom property values, keyed by definition key.
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[schema(as = InvoiceEventData)]
#[from_owned(InvoiceEvent)]
pub struct WebhookOutInvoiceEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub invoice_id: InvoiceId,
    /// Absent while the invoice is a draft — the number is assigned at finalization.
    pub invoice_number: Option<String>,
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    #[from(~.into())]
    pub status: InvoiceStatus,
    pub currency: String,
    pub total: i64,
    pub tax_amount: i64,
    #[from(~.into())]
    pub created_at: ApiDateTime,
    /// Set on `invoice.consolidated`: the parent invoice this one was merged into.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub consolidated_into_invoice_id: Option<InvoiceId>,
    /// Set on a corrective invoice: the cancelled invoice this one replaces
    /// ("cancels and replaces"). The original is credited in full by a credit note.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub parent_invoice_id: Option<InvoiceId>,
    /// User-defined custom property values, keyed by definition key.
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
    /// On `invoice.paid`: true when finalized credit notes, not a payment, settled the invoice's
    /// whole total. Nothing was collected, so it books no cash. False on every other event.
    pub settled_by_credit_notes: bool,
}

/// Identifies which mutation triggered a `subscription.updated` webhook.
#[derive(Clone, Copy, Debug, Serialize, o2o, utoipa::ToSchema)]
#[from_owned(meteroid_store::domain::outbox_event::SubscriptionUpdateType)]
#[schema(as = SubscriptionUpdateType)]
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
    #[from(~.into())]
    pub created_at: ApiDateTime,
    pub net_terms: u32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    #[from(~.map(Into::into))]
    pub activated_at: Option<ApiDateTime>,
    pub mrr_cents: u64,
    #[from(~.into())]
    pub period: BillingPeriodEnum,
    #[from(~.into())]
    pub status: SubscriptionStatusEnum,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub purchase_order: Option<String>,
    /// Present on `subscription.updated`: which mutation fired the event.
    #[from(~.map(Into::into))]
    pub change_type: Option<SubscriptionUpdateType>,
    /// Present on `subscription.cancelled` when a reason was supplied.
    pub cancellation_reason: Option<String>,
    /// User-defined custom property values, keyed by definition key.
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
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
    /// What one unit of the metric is called, e.g. `token`.
    pub unit_singular: Option<String>,
    /// What several units of the metric are called, e.g. `tokens`.
    pub unit_plural: Option<String>,
    #[from(~.into())]
    pub created_at: ApiDateTime,
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

// Same variants as the credit notes API — re-exported so both emit a single
// `CreditNoteStatus` OpenAPI schema. The `From<domain::CreditNoteStatus>` impl comes from o2o.
pub use crate::creditnotes::model::CreditNoteStatus;

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[schema(as = CreditNoteEventData)]
#[from_owned(CreditNoteEvent)]
pub struct WebhookOutCreditNoteEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub credit_note_id: CreditNoteId,
    /// Absent while the credit note is a draft — the number is assigned at finalization.
    pub credit_note_number: Option<String>,
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub invoice_id: InvoiceId,
    /// Number of the invoice being credited.
    pub invoice_number: Option<String>,
    #[from(~.into())]
    pub status: CreditNoteStatus,
    pub currency: String,
    pub subtotal: i64,
    pub total: i64,
    pub tax_amount: i64,
    pub refunded_amount_cents: i64,
    pub credited_amount_cents: i64,
    /// Credited line items (negated amounts).
    #[from(~.into_iter().map(map_line_item_to_rest).collect())]
    pub line_items: Vec<InvoiceLineItem>,
    /// Per-rate tax (VAT) breakdown for the credited amount.
    #[from(~.into_iter().map(map_tax_breakdown_to_rest).collect())]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub reason: Option<String>,
    pub memo: Option<String>,
    /// User-defined custom property values, keyed by definition key.
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
    #[from(~.into())]
    pub created_at: ApiDateTime,
}

/// A refund row, or the payment that failed or was clawed back — the same shape the REST invoice
/// exposes as a transaction. `reversal_reason` is deliberately absent: it is free provider text and is
/// not carried on the outbox event.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = RefundEventData)]
pub struct WebhookOutRefundEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub transaction_id: PaymentTransactionId,
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub invoice_id: Option<InvoiceId>,
    pub provider_transaction_id: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub status: PaymentStatusEnum,
    pub payment_type: PaymentTypeEnum,
    pub error: Option<String>,
    pub decline_kind: Option<DeclineKind>,
    pub processed_at: Option<ApiDateTime>,
    pub amount_refunded: i64,
    pub amount_reversed: i64,
    /// The part of `amount_reversed` lost to disputes or chargebacks: the invoice stays paid.
    #[serde(default)]
    pub amount_disputed: i64,
    /// What the provider withdrew above what the payment still held (refunded before); never
    /// applied to the invoice or credit. Contest it with the provider.
    #[serde(default)]
    pub amount_reversed_excess: i64,
    pub reversal_kind: Option<ReversalKind>,
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub parent_transaction_id: Option<PaymentTransactionId>,
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub credit_note_id: Option<CreditNoteId>,
    pub refund_mode: Option<RefundMode>,
}

impl From<PaymentTransactionEvent> for WebhookOutRefundEventData {
    fn from(event: PaymentTransactionEvent) -> Self {
        Self {
            transaction_id: event.payment_transaction_id,
            invoice_id: event.invoice_id,
            provider_transaction_id: event.provider_transaction_id,
            amount: event.amount,
            currency: event.currency,
            status: map_payment_status_enum_to_rest(event.status),
            payment_type: map_payment_type_to_rest(event.payment_type),
            error: event.error_type,
            decline_kind: event.decline_kind.map(map_decline_kind_to_rest),
            processed_at: event.processed_at.map(Into::into),
            amount_refunded: event.amount_refunded,
            amount_reversed: event.amount_reversed,
            amount_disputed: event.amount_disputed,
            amount_reversed_excess: event.amount_reversed_excess,
            reversal_kind: event.reversal_kind.map(map_reversal_kind_to_rest),
            parent_transaction_id: event.parent_transaction_id,
            credit_note_id: event.credit_note_id,
            refund_mode: event.refund_mode.map(map_refund_mode_to_rest),
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = PaymentEvent)]
pub struct WebhookOutPaymentEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutRefundEventData,
    pub timestamp: ApiDateTime,
}

pub type PlanTypeEnum = RestPlanTypeEnum;
pub type PlanStatusEnum = RestPlanStatusEnum;

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = PlanEventData)]
pub struct WebhookOutPlanEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub plan_id: PlanId,
    pub name: String,
    pub description: Option<String>,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub currency: String,
    pub version: i32,
    pub created_at: ApiDateTime,
}

impl From<PlanEvent> for WebhookOutPlanEventData {
    fn from(e: PlanEvent) -> Self {
        Self {
            plan_id: e.plan_id,
            name: e.name,
            description: e.description,
            plan_type: e.plan_type.into(),
            status: e.status.into(),
            currency: e.currency,
            version: e.version,
            created_at: e.created_at.into(),
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[schema(as = ProductEventData)]
#[from_owned(ProductEvent)]
pub struct WebhookOutProductEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub product_id: ProductId,
    pub name: String,
    pub description: Option<String>,
    #[from(~.into())]
    pub fee_type: ProductFeeTypeEnum,
    #[serde(serialize_with = "string_serde::serialize")]
    pub product_family_id: ProductFamilyId,
    #[from(~.into())]
    pub created_at: ApiDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = CouponEventData)]
pub struct WebhookOutCouponEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub coupon_id: CouponId,
    pub code: String,
    pub description: String,
    pub discount: CouponDiscount,
    pub expires_at: Option<ApiDateTime>,
    pub redemption_limit: Option<i32>,
    pub recurring_value: Option<i32>,
    pub reusable: bool,
    pub disabled: bool,
    pub created_at: ApiDateTime,
}

impl From<CouponEvent> for WebhookOutCouponEventData {
    fn from(e: CouponEvent) -> Self {
        let discount = crate::coupons::mapping::coupon_discount_to_rest(&e.discount);
        Self {
            coupon_id: e.coupon_id,
            code: e.code,
            description: e.description,
            discount,
            expires_at: e.expires_at.map(Into::into),
            redemption_limit: e.redemption_limit,
            recurring_value: e.recurring_value,
            reusable: e.reusable,
            disabled: e.disabled,
            created_at: e.created_at.into(),
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = AddOnEventData)]
pub struct WebhookOutAddOnEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub add_on_id: AddOnId,
    pub name: String,
    pub description: Option<String>,
    #[serde(serialize_with = "string_serde::serialize")]
    pub product_id: ProductId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub price_id: PriceId,
    pub fee_type: Option<ProductFeeTypeEnum>,
    pub self_serviceable: bool,
    pub max_instances_per_subscription: Option<i32>,
    pub created_at: ApiDateTime,
}

impl From<AddOnEvent> for WebhookOutAddOnEventData {
    fn from(e: AddOnEvent) -> Self {
        Self {
            add_on_id: e.add_on_id,
            name: e.name,
            description: e.description,
            product_id: e.product_id,
            price_id: e.price_id,
            fee_type: e.fee_type.map(Into::into),
            self_serviceable: e.self_serviceable,
            max_instances_per_subscription: e.max_instances_per_subscription,
            created_at: e.created_at.into(),
        }
    }
}

pub use crate::credits::model::{CreditGrantSource, CreditGrantType};

/// Who made a change.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = Actor)]
pub struct WebhookOutActor {
    /// USER, CUSTOMER, API_KEY, OAUTH_APP or SYSTEM.
    #[serde(rename = "type")]
    pub actor_type: WebhookOutActorType,
    /// The user's id (`usr_…`) for USER, the customer's (`cus_…`) for CUSTOMER, the API key's
    /// or app's for API_KEY and OAUTH_APP; absent for SYSTEM.
    pub id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(as = ActorType)]
pub enum WebhookOutActorType {
    /// A user of your Meteroid dashboard.
    User,
    /// The customer, from the customer portal.
    Customer,
    /// Meteroid on its own, or one of your API keys where the change does not name it.
    System,
    /// One of your API keys.
    ApiKey,
    /// An app you connected.
    OauthApp,
}

impl WebhookOutActor {
    fn user(id: UserId) -> Self {
        Self {
            actor_type: WebhookOutActorType::User,
            id: Some(id.as_base62()),
        }
    }

    fn system() -> Self {
        Self {
            actor_type: WebhookOutActorType::System,
            id: None,
        }
    }

    /// Reads the store's `user:<uuid>` / `customer:<id>` / `system` form.
    fn parse(actor: &str) -> Self {
        let parsed = match actor.split_once(':') {
            Some(("user", id)) => UserId::from_str(id).ok().map(Self::user),
            Some(("customer", id)) => CustomerId::from_str(id).ok().map(|id| Self {
                actor_type: WebhookOutActorType::Customer,
                id: Some(id.as_base62()),
            }),
            Some(("api_token", id)) => uuid::Uuid::parse_str(id).ok().map(|id| Self {
                actor_type: WebhookOutActorType::ApiKey,
                id: Some(common_domain::ids::ApiTokenId::from(id).as_base62()),
            }),
            Some(("oauth_app", id)) => uuid::Uuid::parse_str(id).ok().map(|id| Self {
                actor_type: WebhookOutActorType::OauthApp,
                id: Some(common_domain::ids::OAuthAppId::from(id).as_base62()),
            }),
            None if actor == "system" => Some(Self::system()),
            _ => None,
        };
        parsed.unwrap_or_else(|| {
            log::error!("Unrecognized wallet event actor {actor:?}");
            Self::system()
        })
    }
}

/// A wallet amount's code as `(currency, credit_unit)`: an event reports a unit's precision.
fn split_denomination(
    code: String,
    unit_precision: Option<u8>,
) -> (Option<String>, Option<String>) {
    match unit_precision {
        Some(_) => (None, Some(code)),
        None => (Some(code), None),
    }
}

/// Credit entered a customer's wallet.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = WalletFundedEventData)]
pub struct WebhookOutWalletFundedEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// What the grant pays: ALL any charge, USAGE only usage charges.
    pub applies_to: crate::credits::model::CreditAppliesTo,
    /// The subscription the grant belongs to: it pays only that subscription's charges. Null when
    /// it pays any of the customer's subscriptions.
    #[serialize_always]
    #[serde(serialize_with = "string_serde_opt::serialize")]
    #[schema(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
    /// The products whose charges the grant pays; empty when it pays every product.
    #[serde(serialize_with = "string_serde_vec::serialize")]
    #[schema(value_type = Vec<String>)]
    pub applicable_product_ids: Vec<ProductId>,
    /// The credit ledger entry recording the movement.
    #[serde(serialize_with = "string_serde::serialize")]
    pub ledger_entry_id: LedgerEntryId,
    /// The grant the credit entered.
    #[serde(serialize_with = "string_serde::serialize")]
    pub credit_grant_id: CreditGrantId,
    /// The invoice that bought the credit, or whose void gave it back.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub invoice_id: Option<InvoiceId>,
    /// The credit note that gave the credit back.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub credit_note_id: Option<CreditNoteId>,
    /// In minor units of `currency` or `credit_unit`.
    pub amount: i64,
    /// Absent when the amounts are in a credit unit.
    pub currency: Option<String>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// Where the credit came from.
    pub source: CreditGrantSource,
    /// PAID or PROMOTIONAL: whether the credit applies after or before tax.
    pub grant_type: CreditGrantType,
}

impl From<WalletFundedEvent> for WebhookOutWalletFundedEventData {
    fn from(event: WalletFundedEvent) -> Self {
        let (currency, credit_unit) =
            split_denomination(event.currency, event.credit_unit_precision);
        Self {
            customer_id: event.customer_id,
            applies_to: event.application.into(),
            subscription_id: event.subscription_id,
            applicable_product_ids: event.product_ids,
            ledger_entry_id: event.ledger_entry_id,
            credit_grant_id: event.credit_grant_id,
            invoice_id: event.invoice_id,
            credit_note_id: event.credit_note_id,
            amount: event.amount_minor,
            currency,
            credit_unit,
            credit_unit_precision: event.credit_unit_precision,
            source: event.source.into(),
            grant_type: event.classification.into(),
        }
    }
}

/// Credit left a customer's wallet.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = WalletDebitedEventData)]
pub struct WebhookOutWalletDebitedEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// What the grant pays: ALL any charge, USAGE only usage charges.
    pub applies_to: crate::credits::model::CreditAppliesTo,
    /// The subscription the grant belongs to: it pays only that subscription's charges. Null when
    /// it pays any of the customer's subscriptions.
    #[serialize_always]
    #[serde(serialize_with = "string_serde_opt::serialize")]
    #[schema(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
    /// The products whose charges the grant pays; empty when it pays every product.
    #[serde(serialize_with = "string_serde_vec::serialize")]
    #[schema(value_type = Vec<String>)]
    pub applicable_product_ids: Vec<ProductId>,
    /// The credit ledger entry recording the movement.
    #[serde(serialize_with = "string_serde::serialize")]
    pub ledger_entry_id: LedgerEntryId,
    /// The grant the credit left.
    #[serde(serialize_with = "string_serde::serialize")]
    pub credit_grant_id: CreditGrantId,
    /// The invoice the credit was applied to.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub invoice_id: Option<InvoiceId>,
    /// The credit note that took the credit back.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub credit_note_id: Option<CreditNoteId>,
    /// In minor units of `currency` or `credit_unit`.
    pub amount: i64,
    /// Absent when the amounts are in a credit unit.
    pub currency: Option<String>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// Why the credit moved, as shown on the ledger.
    pub reason: String,
    /// Who removed the credit by hand. Absent when an invoice, a credit note or a reversed
    /// payment moved it.
    pub actor: Option<WebhookOutActor>,
}

impl From<WalletDebitedEvent> for WebhookOutWalletDebitedEventData {
    fn from(event: WalletDebitedEvent) -> Self {
        let (currency, credit_unit) =
            split_denomination(event.currency, event.credit_unit_precision);
        Self {
            customer_id: event.customer_id,
            applies_to: event.application.into(),
            subscription_id: event.subscription_id,
            applicable_product_ids: event.product_ids,
            ledger_entry_id: event.ledger_entry_id,
            credit_grant_id: event.credit_grant_id,
            invoice_id: event.invoice_id,
            credit_note_id: event.credit_note_id,
            amount: event.amount_minor,
            currency,
            credit_unit,
            credit_unit_precision: event.credit_unit_precision,
            reason: event.reason,
            actor: event
                .actor_id
                .map(|id| WebhookOutActor::user(UserId::from(id))),
        }
    }
}

/// A credit grant expired, or the subscription it belongs to ended, with credit left, which
/// lapsed.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = CreditGrantExpiredEventData)]
pub struct WebhookOutCreditGrantExpiredEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// What the grant pays: ALL any charge, USAGE only usage charges.
    pub applies_to: crate::credits::model::CreditAppliesTo,
    /// The subscription the grant belongs to: it pays only that subscription's charges. Null when
    /// it pays any of the customer's subscriptions.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    #[schema(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
    /// The products whose charges the grant pays; empty when it pays every product.
    #[serde(serialize_with = "string_serde_vec::serialize")]
    #[schema(value_type = Vec<String>)]
    pub applicable_product_ids: Vec<ProductId>,
    /// The grant that expired.
    #[serde(serialize_with = "string_serde::serialize")]
    pub credit_grant_id: CreditGrantId,
    /// The credit ledger entry recording the expiry.
    #[serde(serialize_with = "string_serde::serialize")]
    pub ledger_entry_id: LedgerEntryId,
    /// Unspent credit that lapsed, in minor units of `currency` or `credit_unit`.
    pub expired_amount: i64,
    /// Absent when the amounts are in a credit unit.
    pub currency: Option<String>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// When the grant expired, or its subscription ended.
    pub expired_at: ApiDateTime,
    /// SUBSCRIPTION_ENDED when the subscription the grant belongs to ended at or before the grant's
    /// expiry, EXPIRED otherwise.
    pub reason: CreditGrantLapseReason,
}

/// Why a credit grant's unspent credit lapsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditGrantLapseReason {
    /// It reached its expiry.
    Expired,
    /// The subscription it belongs to ended at or before its expiry.
    SubscriptionEnded,
}

impl From<CreditGrantExpiredEvent> for WebhookOutCreditGrantExpiredEventData {
    fn from(event: CreditGrantExpiredEvent) -> Self {
        let (currency, credit_unit) =
            split_denomination(event.currency, event.credit_unit_precision);
        Self {
            customer_id: event.customer_id,
            applies_to: event.application.into(),
            subscription_id: event.subscription_id,
            applicable_product_ids: event.product_ids,
            reason: match event.reason {
                GrantLapseReason::Expired => CreditGrantLapseReason::Expired,
                GrantLapseReason::SubscriptionEnded => CreditGrantLapseReason::SubscriptionEnded,
            },
            credit_grant_id: event.credit_grant_id,
            ledger_entry_id: event.ledger_entry_id,
            expired_amount: event.expired_minor,
            currency,
            credit_unit,
            credit_unit_precision: event.credit_unit_precision,
            expired_at: event.expired_at.into(),
        }
    }
}

/// A wallet's balance fell below one of its low-balance thresholds.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = WalletBalanceLowEventData)]
pub struct WebhookOutWalletBalanceLowEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// Absent when the amounts are in a credit unit.
    pub currency: Option<String>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// The balance when it fell below `threshold`, in minor units: the wallet's `balance`, all
    /// its credit on account net of the unbilled usage it pays, including credit that belongs to
    /// a subscription or is limited to usage or products. Sent once per crossing, for the lowest
    /// threshold crossed; it re-arms once the balance is back at the threshold. Because that
    /// credit counts here and not in `wallet.depleted`, a wallet can be depleted without this
    /// event firing.
    pub balance: i64,
    /// The low-balance threshold crossed, in the same minor units.
    pub threshold: i64,
    /// The part of `balance` that pays any charge of any subscription: what automatic top-ups
    /// refill and `wallet.depleted` watches. Absent on events recorded before it was reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_balance: Option<i64>,
}

impl From<WalletBalanceLowEvent> for WebhookOutWalletBalanceLowEventData {
    fn from(event: WalletBalanceLowEvent) -> Self {
        let (currency, credit_unit) =
            split_denomination(event.currency, event.credit_unit_precision);
        Self {
            customer_id: event.customer_id,
            currency,
            credit_unit,
            credit_unit_precision: event.credit_unit_precision,
            balance: event.available_minor,
            threshold: event.threshold_minor,
            shared_balance: event.shared_minor,
        }
    }
}

/// A wallet's balance (credit that pays any charge of any of the customer's subscriptions, net of
/// unbilled usage) reached zero; credit included with a subscription may remain.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = WalletDepletedEventData)]
pub struct WebhookOutWalletDepletedEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// Absent when the amounts are in a credit unit.
    pub currency: Option<String>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
}

impl From<WalletDepletedEvent> for WebhookOutWalletDepletedEventData {
    fn from(event: WalletDepletedEvent) -> Self {
        let (currency, credit_unit) =
            split_denomination(event.currency, event.credit_unit_precision);
        Self {
            customer_id: event.customer_id,
            currency,
            credit_unit,
            credit_unit_precision: event.credit_unit_precision,
        }
    }
}

/// A wallet's balance (credit that pays any charge of any of the customer's subscriptions, net of
/// unbilled usage) came back above zero.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = WalletRecoveredEventData)]
pub struct WebhookOutWalletRecoveredEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// Absent when the amounts are in a credit unit.
    pub currency: Option<String>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// The balance (credit on account that pays any charge of any of the customer's subscriptions,
    /// net of unbilled usage) now that it is back above zero, in
    /// minor units.
    pub balance: i64,
}

impl From<WalletRecoveredEvent> for WebhookOutWalletRecoveredEventData {
    fn from(event: WalletRecoveredEvent) -> Self {
        let (currency, credit_unit) =
            split_denomination(event.currency, event.credit_unit_precision);
        Self {
            customer_id: event.customer_id,
            currency,
            credit_unit,
            credit_unit_precision: event.credit_unit_precision,
            balance: event.available_minor,
        }
    }
}

/// Purchased credit was already spent when its money came back.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = WalletCreditUnrecoveredEventData)]
pub struct WebhookOutWalletCreditUnrecoveredEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// The invoice whose money bought the credit; absent for a payment no invoice took (a
    /// plan-change payment kept as credit).
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub invoice_id: Option<InvoiceId>,
    /// The payment whose reversal or reinstatement moved the loss; always set when `invoice_id`
    /// is absent.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub payment_id: Option<PaymentTransactionId>,
    /// The refund credit note that caused the loss; absent for a reversed payment.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub credit_note_id: Option<CreditNoteId>,
    /// `PAYMENT_DISPUTED` (lost to a dispute or chargeback), `PAYMENT_REVERSED` (a failed
    /// collection clawed back), `PAYMENT_REFUNDED` (a payment refunded without a credit note, at
    /// the provider or as an overpayment, after the credit it backed was spent) or
    /// `CREDIT_NOTE_REFUNDED`.
    pub cause: String,
    /// Absent when the amounts are in a credit unit.
    pub currency: Option<String>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// Unspent credit taken back from the wallet at the same time, in minor units.
    pub credit_taken_back: i64,
    /// Spent credit this event adds to the loss, in minor units. A failed collection reopens its
    /// invoice, which collects it again; a dispute's loss stays unless the dispute is won.
    /// `wallet.credit_recovered` reports what comes back.
    pub credit_unrecovered: i64,
    /// The loss of this money flow (an invoice's payments, a credit note, or a payment) once
    /// this event applies.
    pub credit_unrecovered_total: i64,
}

impl From<WalletCreditUnrecoveredEvent> for WebhookOutWalletCreditUnrecoveredEventData {
    fn from(event: WalletCreditUnrecoveredEvent) -> Self {
        let (currency, credit_unit) =
            split_denomination(event.currency, event.credit_unit_precision);
        Self {
            customer_id: event.customer_id,
            invoice_id: event.invoice_id,
            payment_id: event.payment_id,
            credit_note_id: event.credit_note_id,
            cause: event.cause.as_str().to_owned(),
            currency,
            credit_unit,
            credit_unit_precision: event.credit_unit_precision,
            credit_taken_back: event.taken_back_minor,
            credit_unrecovered: event.unrecovered_minor,
            credit_unrecovered_total: event.unrecovered_total_minor,
        }
    }
}

/// Spent credit reported by `wallet.credit_unrecovered` is backed again.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = WalletCreditRecoveredEventData)]
pub struct WebhookOutWalletCreditRecoveredEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// The invoice whose money bought the credit; absent for a payment no invoice took.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub invoice_id: Option<InvoiceId>,
    /// The payment whose reinstatement recovered it, if one did; always set when `invoice_id`
    /// is absent.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub payment_id: Option<PaymentTransactionId>,
    /// The refund credit note behind the loss; absent for a reversed payment.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub credit_note_id: Option<CreditNoteId>,
    /// Cause of the loss this recovers: `PAYMENT_DISPUTED`, `PAYMENT_REVERSED`,
    /// `PAYMENT_REFUNDED` or `CREDIT_NOTE_REFUNDED`.
    pub cause: String,
    /// Absent when the amounts are in a credit unit.
    pub currency: Option<String>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// Spent credit an earlier `wallet.credit_unrecovered` reported that is backed again, in
    /// minor units: the money came back, or the credit was returned and taken back.
    pub credit_recovered: i64,
    /// The loss of this money flow (an invoice's payments, a credit note, or a payment) once
    /// this event applies.
    pub credit_unrecovered_total: i64,
}

impl From<WalletCreditRecoveredEvent> for WebhookOutWalletCreditRecoveredEventData {
    fn from(event: WalletCreditRecoveredEvent) -> Self {
        let (currency, credit_unit) =
            split_denomination(event.currency, event.credit_unit_precision);
        Self {
            customer_id: event.customer_id,
            invoice_id: event.invoice_id,
            payment_id: event.payment_id,
            credit_note_id: event.credit_note_id,
            cause: event.cause.as_str().to_owned(),
            currency,
            credit_unit,
            credit_unit_precision: event.credit_unit_precision,
            credit_recovered: event.recovered_minor,
            credit_unrecovered_total: event.unrecovered_total_minor,
        }
    }
}

/// An automatic top-up charge failed, or the automatic top-up was suspended.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = AutoTopUpEventData)]
pub struct WebhookOutAutoTopUpEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// The currency charged (ISO 4217), or the credit unit code when the credits had no price.
    pub currency: String,
    /// The charge in minor units of `currency`: the credit it would have added, or for a credit
    /// unit wallet the price of those credits in the customer's currency. When the credits could
    /// not be priced (no selling price), the credits themselves, with `currency` the unit code. On
    /// a suspension after a top-up payment was refunded or disputed: that payment's top-up.
    pub amount: i64,
    /// Failed charges in a row, this one included.
    pub consecutive_failures: i32,
    /// Set on `wallet.auto_top_up_suspended`: when it was suspended.
    pub suspended_at: Option<ApiDateTime>,
    /// May carry payment provider detail.
    pub error: String,
}

impl From<WalletAutoTopUpEvent> for WebhookOutAutoTopUpEventData {
    fn from(event: WalletAutoTopUpEvent) -> Self {
        Self {
            customer_id: event.customer_id,
            currency: event.currency,
            amount: event.amount_minor,
            consecutive_failures: event.consecutive_failures,
            suspended_at: event.suspended_at.map(Into::into),
            error: event.error,
        }
    }
}

/// The authorization of a wallet's automatic top-up was given, changed or withdrawn.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = AutoTopUpConsentChangedEventData)]
pub struct WebhookOutAutoTopUpConsentChangedEventData {
    /// The wallet's currency (ISO 4217) or credit unit code.
    pub currency: String,
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// Whether automatic top-up is authorized after this change.
    pub enabled: bool,
    /// Identifies the terms authorized before the change; absent when none were.
    pub previous_configuration_revision: Option<String>,
    /// Identifies the terms authorized now, without disclosing payment details; absent when
    /// withdrawn.
    pub configuration_revision: Option<String>,
    /// Who gave, changed or withdrew the authorization.
    pub actor: WebhookOutActor,
    /// When the authorization changed.
    pub changed_at: ApiDateTime,
    /// Terms set by you rather than confirmed by the customer: who attested that the customer
    /// agreed to automatic charges on them.
    pub consent_attested_by: Option<WebhookOutActor>,
    /// When that attestation was made.
    pub consent_attested_at: Option<ApiDateTime>,
}

impl From<WalletAutoTopUpConsentChangedEvent> for WebhookOutAutoTopUpConsentChangedEventData {
    fn from(event: WalletAutoTopUpConsentChangedEvent) -> Self {
        Self {
            currency: event.currency,
            customer_id: event.customer_id,
            enabled: event.enabled,
            previous_configuration_revision: event.previous_configuration_revision,
            configuration_revision: event.configuration_revision,
            actor: WebhookOutActor::parse(&event.actor),
            changed_at: event.changed_at.into(),
            consent_attested_by: event
                .consent_attested_by
                .as_deref()
                .map(WebhookOutActor::parse),
            consent_attested_at: event.consent_attested_at.map(Into::into),
        }
    }
}

/// A spending cap stopped an automatic top-up.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = AutoTopUpCapReachedEventData)]
pub struct WebhookOutAutoTopUpCapReachedEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    /// The currency the cap and the charge are in (ISO 4217).
    pub currency: String,
    /// `DAILY` (rolling day) or `MONTHLY` (calendar month, UTC).
    pub period: String,
    /// The cap, tax included, in minor units of `currency`.
    pub cap: i64,
    /// What the suppressed top-up would have charged, tax included once it was quoted (before
    /// tax when the cap stopped it before then). Caps are always money: for a credit unit wallet,
    /// `currency` is the customer's currency.
    pub attempted_amount: i64,
    /// Start of the capped period.
    pub period_start: ApiDateTime,
    /// End of the capped period.
    pub period_end: ApiDateTime,
}

impl From<WalletAutoTopUpCapReachedEvent> for WebhookOutAutoTopUpCapReachedEventData {
    fn from(event: WalletAutoTopUpCapReachedEvent) -> Self {
        Self {
            customer_id: event.customer_id,
            currency: event.currency,
            period: event.period.as_str().to_owned(),
            cap: event.cap_minor,
            attempted_amount: event.attempted_amount_minor,
            period_start: event.period_start.into(),
            period_end: event.period_end.into(),
        }
    }
}

/// The period an alert was measured over; `end` is absent for a period that never ends.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = AlertPeriod)]
pub struct WebhookOutAlertPeriod {
    pub start: ApiDateTime,
    pub end: Option<ApiDateTime>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = EntitlementAlertEventData)]
pub struct WebhookOutEntitlementAlertEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    pub customer_alias: Option<String>,
    #[serde(serialize_with = "string_serde::serialize")]
    pub feature_id: FeatureId,
    /// Absent for a customer without an active subscription.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    #[schema(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
    pub metric_code: String,
    /// Dimension values of the group that crossed, when the metric is grouped.
    pub group: Option<std::collections::BTreeMap<String, String>>,
    /// Percentage of the limit the notification is for; 100 is the limit itself.
    /// Several levels crossed at once are sent in ascending order, but delivery order is not
    /// guaranteed (retries): rely on this field, not on arrival order.
    pub threshold_pct: u32,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub used: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub limit: rust_decimal::Decimal,
    pub period: WebhookOutAlertPeriod,
    pub as_of: ApiDateTime,
    /// Always false: usage is aggregated asynchronously and may lag the latest events.
    pub exact: bool,
}

impl From<EntitlementAlertEvent> for WebhookOutEntitlementAlertEventData {
    fn from(event: EntitlementAlertEvent) -> Self {
        Self {
            customer_id: event.customer_id,
            customer_alias: event.customer_alias,
            feature_id: event.feature_id,
            subscription_id: event.subscription_id,
            metric_code: event.metric_code,
            group: event.group,
            threshold_pct: event.threshold_pct,
            used: event.used,
            limit: event.limit,
            period: WebhookOutAlertPeriod {
                start: event.window_start.into(),
                end: event.window_end.map(Into::into),
            },
            as_of: event.as_of.into(),
            exact: false,
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = BudgetAlertEventData)]
pub struct WebhookOutBudgetAlertEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    pub customer_alias: Option<String>,
    #[serde(serialize_with = "string_serde::serialize")]
    pub budget_id: BudgetId,
    /// Set when the budget counts only this subscription's usage.
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub subscription_id: Option<SubscriptionId>,
    /// ISO 4217 code of the budget's currency; absent for a budget in a credit unit.
    pub currency: Option<String>,
    /// The credit unit (its code) the budget counts; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// Percentage of the budget the notification is for; 100 is the budget itself.
    /// Several levels crossed at once are sent in ascending order, but delivery order is not
    /// guaranteed (retries): rely on this field, not on arrival order.
    pub threshold_pct: u32,
    /// Spend of the period, in minor units, including usage not invoiced yet.
    pub used: i64,
    /// The budget, in minor units of its currency or credit unit.
    pub amount: i64,
    pub period: WebhookOutAlertPeriod,
    pub as_of: ApiDateTime,
    /// Always false: spend is computed from usage aggregated asynchronously.
    pub exact: bool,
}

impl From<BudgetAlertEvent> for WebhookOutBudgetAlertEventData {
    fn from(event: BudgetAlertEvent) -> Self {
        Self {
            customer_id: event.customer_id,
            customer_alias: event.customer_alias,
            budget_id: event.budget_id,
            subscription_id: event.subscription_id,
            currency: (event.currency.len() == 3).then(|| event.currency.clone()),
            credit_unit: (event.currency.len() != 3).then(|| event.currency.clone()),
            credit_unit_precision: event.credit_unit_precision,
            threshold_pct: event.threshold_pct,
            used: event.used_minor,
            amount: event.amount_minor,
            period: WebhookOutAlertPeriod {
                start: event.window_start.into(),
                end: event.window_end.map(Into::into),
            },
            as_of: event.as_of.into(),
            exact: false,
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = TooManyGroupsEventData)]
pub struct WebhookOutTooManyGroupsEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    pub customer_alias: Option<String>,
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub feature_id: Option<FeatureId>,
    #[serde(serialize_with = "string_serde_opt::serialize")]
    pub subscription_id: Option<SubscriptionId>,
    pub metric_code: String,
    /// The property the limit is grouped by.
    pub group_by: String,
    /// How many groups the limit keeps; callers of any other group are refused
    /// (`TOO_MANY_GROUPS`) until the period ends or a group is retired.
    pub max_groups: u32,
    pub period: WebhookOutAlertPeriod,
    pub as_of: ApiDateTime,
}

impl From<TooManyGroupsEvent> for WebhookOutTooManyGroupsEventData {
    fn from(event: TooManyGroupsEvent) -> Self {
        Self {
            customer_id: event.customer_id,
            customer_alias: event.customer_alias,
            feature_id: event.feature_id,
            subscription_id: event.subscription_id,
            metric_code: event.metric_code,
            group_by: event.group_by,
            max_groups: event.max_groups,
            period: WebhookOutAlertPeriod {
                start: event.window_start.into(),
                end: event.window_end.map(Into::into),
            },
            as_of: event.as_of.into(),
        }
    }
}

macro_rules! wallet_webhook_event {
    ($($event:ident($data:ty) as $schema:ident),+ $(,)?) => {$(
        #[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
        #[schema(as = $schema)]
        pub struct $event {
            #[serde(serialize_with = "string_serde::serialize")]
            pub id: EventId,
            #[serde(rename = "type")]
            pub event_type: WebhookOutEventTypeEnum,
            #[serde(flatten)]
            pub data: $data,
            pub timestamp: ApiDateTime,
        }

    )+};
}

wallet_webhook_event!(
    WebhookOutWalletFundedEvent(WebhookOutWalletFundedEventData) as WalletFundedEvent,
    WebhookOutWalletDebitedEvent(WebhookOutWalletDebitedEventData) as WalletDebitedEvent,
    WebhookOutCreditGrantExpiredEvent(WebhookOutCreditGrantExpiredEventData)
        as CreditGrantExpiredEvent,
    WebhookOutWalletBalanceLowEvent(WebhookOutWalletBalanceLowEventData) as WalletBalanceLowEvent,
    WebhookOutWalletDepletedEvent(WebhookOutWalletDepletedEventData) as WalletDepletedEvent,
    WebhookOutWalletRecoveredEvent(WebhookOutWalletRecoveredEventData) as WalletRecoveredEvent,
    WebhookOutWalletCreditUnrecoveredEvent(WebhookOutWalletCreditUnrecoveredEventData)
        as WalletCreditUnrecoveredEvent,
    WebhookOutWalletCreditRecoveredEvent(WebhookOutWalletCreditRecoveredEventData)
        as WalletCreditRecoveredEvent,
    WebhookOutAutoTopUpEvent(WebhookOutAutoTopUpEventData) as AutoTopUpEvent,
    WebhookOutAutoTopUpConsentChangedEvent(WebhookOutAutoTopUpConsentChangedEventData)
        as AutoTopUpConsentChangedEvent,
    WebhookOutAutoTopUpCapReachedEvent(WebhookOutAutoTopUpCapReachedEventData)
        as AutoTopUpCapReachedEvent,
    WebhookOutEntitlementAlertEvent(WebhookOutEntitlementAlertEventData)
        as EntitlementAlertWebhookEvent,
    WebhookOutBudgetAlertEvent(WebhookOutBudgetAlertEventData) as BudgetAlertWebhookEvent,
    WebhookOutTooManyGroupsEvent(WebhookOutTooManyGroupsEventData) as TooManyGroupsWebhookEvent,
);

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
    pub timestamp: ApiDateTime,
}

/// One rule the document did not satisfy, in the standard's own vocabulary.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = EInvoicingFinding)]
pub struct WebhookOutEInvoicingFinding {
    /// The rule identifier — "BR-11", "PEPPOL-EN16931-R003".
    pub rule: String,
    /// The business term path it is about — "BG-8/BT-55".
    pub term: String,
    pub message: String,
    pub hint: Option<String>,
}

/// Emitted once the accounting PDF is stored. This is also the moment the e-invoicing
/// outcome is known: the structured document is produced with the PDF, not at finalization.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = InvoiceDocumentsEventData)]
pub struct WebhookOutInvoiceDocumentsEventData {
    #[serde(serialize_with = "string_serde::serialize")]
    pub invoice_id: InvoiceId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub customer_id: CustomerId,
    pub pdf_document_id: String,
    /// The structured e-invoice stored beside the PDF, when one was produced.
    pub xml_document_id: Option<String>,
    /// Absent when the invoicing entity had not opted in.
    pub einvoicing_status: Option<EInvoicingStatus>,
    /// The profile the document was checked against, e.g. "EN 16931".
    pub einvoicing_profile: Option<String>,
    /// Empty unless the status is `failed`.
    pub einvoicing_findings: Vec<WebhookOutEInvoicingFinding>,
    /// Set when generation failed for a reason that is not a business rule.
    pub einvoicing_error: Option<String>,
}

impl From<InvoicePdfGeneratedEvent> for WebhookOutInvoiceDocumentsEventData {
    fn from(event: InvoicePdfGeneratedEvent) -> Self {
        let report = event.einvoicing_report;
        WebhookOutInvoiceDocumentsEventData {
            invoice_id: event.invoice_id,
            customer_id: event.customer_id,
            pdf_document_id: event.pdf_id.to_string(),
            xml_document_id: event.xml_id.map(|id| id.to_string()),
            einvoicing_status: event.einvoicing_status.map(Into::into),
            einvoicing_profile: report.as_ref().map(|r| r.profile.clone()),
            einvoicing_findings: report
                .as_ref()
                .map(|r| {
                    r.findings
                        .iter()
                        .map(|f| WebhookOutEInvoicingFinding {
                            rule: f.rule.clone(),
                            term: f.term.clone(),
                            message: f.message.clone(),
                            hint: f.hint.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            einvoicing_error: report.and_then(|r| r.error),
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = InvoiceDocumentsEvent)]
pub struct WebhookOutInvoiceDocumentsEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutInvoiceDocumentsEventData,
    pub timestamp: ApiDateTime,
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
    pub timestamp: ApiDateTime,
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
    pub timestamp: ApiDateTime,
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
    pub timestamp: ApiDateTime,
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
    pub timestamp: ApiDateTime,
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
    pub timestamp: ApiDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = PlanEvent)]
pub struct WebhookOutPlanEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutPlanEventData,
    pub timestamp: ApiDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = ProductEvent)]
pub struct WebhookOutProductEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutProductEventData,
    pub timestamp: ApiDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = CouponEvent)]
pub struct WebhookOutCouponEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutCouponEventData,
    pub timestamp: ApiDateTime,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = AddOnEvent)]
pub struct WebhookOutAddOnEvent {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: EventId,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    #[serde(flatten)]
    pub data: WebhookOutAddOnEventData,
    pub timestamp: ApiDateTime,
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
    #[strum(serialize = "subscription.updated")]
    #[serde(rename = "subscription.updated")]
    SubscriptionUpdated,
    #[strum(serialize = "subscription.cancelled")]
    #[serde(rename = "subscription.cancelled")]
    SubscriptionCancelled,
    #[strum(serialize = "subscription.ended")]
    #[serde(rename = "subscription.ended")]
    SubscriptionEnded,
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
    #[strum(serialize = "invoice.closed")]
    #[serde(rename = "invoice.closed")]
    InvoiceClosed,
    #[strum(serialize = "invoice.consolidated")]
    #[serde(rename = "invoice.consolidated")]
    InvoiceConsolidated,
    #[strum(serialize = "invoice.deleted")]
    #[serde(rename = "invoice.deleted")]
    InvoiceDeleted,
    #[strum(serialize = "invoice.accounting_pdf_generated")]
    #[serde(rename = "invoice.accounting_pdf_generated")]
    InvoiceAccountingPdfGenerated,
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
    #[strum(serialize = "plan.created")]
    #[serde(rename = "plan.created")]
    PlanCreated,
    #[strum(serialize = "plan.published")]
    #[serde(rename = "plan.published")]
    PlanPublished,
    #[strum(serialize = "plan.archived")]
    #[serde(rename = "plan.archived")]
    PlanArchived,
    #[strum(serialize = "product.created")]
    #[serde(rename = "product.created")]
    ProductCreated,
    #[strum(serialize = "product.updated")]
    #[serde(rename = "product.updated")]
    ProductUpdated,
    #[strum(serialize = "product.archived")]
    #[serde(rename = "product.archived")]
    ProductArchived,
    #[strum(serialize = "metric.updated")]
    #[serde(rename = "metric.updated")]
    BillableMetricUpdated,
    #[strum(serialize = "metric.archived")]
    #[serde(rename = "metric.archived")]
    BillableMetricArchived,
    #[strum(serialize = "coupon.created")]
    #[serde(rename = "coupon.created")]
    CouponCreated,
    #[strum(serialize = "coupon.updated")]
    #[serde(rename = "coupon.updated")]
    CouponUpdated,
    #[strum(serialize = "coupon.archived")]
    #[serde(rename = "coupon.archived")]
    CouponArchived,
    #[strum(serialize = "addon.created")]
    #[serde(rename = "addon.created")]
    AddOnCreated,
    #[strum(serialize = "addon.updated")]
    #[serde(rename = "addon.updated")]
    AddOnUpdated,
    #[strum(serialize = "addon.archived")]
    #[serde(rename = "addon.archived")]
    AddOnArchived,
    #[strum(serialize = "refund.issued")]
    #[serde(rename = "refund.issued")]
    RefundIssued,
    #[strum(serialize = "refund.settled")]
    #[serde(rename = "refund.settled")]
    RefundSettled,
    #[strum(serialize = "refund.failed")]
    #[serde(rename = "refund.failed")]
    RefundFailed,
    #[strum(serialize = "payment.reversed")]
    #[serde(rename = "payment.reversed")]
    PaymentReversed,
    #[strum(serialize = "payment.reinstated")]
    #[serde(rename = "payment.reinstated")]
    PaymentReinstated,
    #[strum(serialize = "payment.failed")]
    #[serde(rename = "payment.failed")]
    PaymentFailed,
    #[strum(serialize = "wallet.funded")]
    #[serde(rename = "wallet.funded")]
    WalletFunded,
    #[strum(serialize = "wallet.debited")]
    #[serde(rename = "wallet.debited")]
    WalletDebited,
    #[strum(serialize = "wallet.balance_low")]
    #[serde(rename = "wallet.balance_low")]
    WalletBalanceLow,
    #[strum(serialize = "wallet.credit_unrecovered")]
    #[serde(rename = "wallet.credit_unrecovered")]
    WalletCreditUnrecovered,
    #[strum(serialize = "wallet.credit_recovered")]
    #[serde(rename = "wallet.credit_recovered")]
    WalletCreditRecovered,
    #[strum(serialize = "wallet.auto_top_up_failed")]
    #[serde(rename = "wallet.auto_top_up_failed")]
    AutoTopUpFailed,
    #[strum(serialize = "wallet.auto_top_up_suspended")]
    #[serde(rename = "wallet.auto_top_up_suspended")]
    AutoTopUpSuspended,
    #[strum(serialize = "wallet.auto_top_up_consent_changed")]
    #[serde(rename = "wallet.auto_top_up_consent_changed")]
    AutoTopUpConsentChanged,
    #[strum(serialize = "wallet.depleted")]
    #[serde(rename = "wallet.depleted")]
    WalletDepleted,
    #[strum(serialize = "wallet.recovered")]
    #[serde(rename = "wallet.recovered")]
    WalletRecovered,
    #[strum(serialize = "wallet.auto_top_up_cap_reached")]
    #[serde(rename = "wallet.auto_top_up_cap_reached")]
    AutoTopUpCapReached,
    #[strum(serialize = "credit_grant.expired")]
    #[serde(rename = "credit_grant.expired")]
    CreditGrantExpired,
    #[strum(serialize = "entitlement.usage_threshold_reached")]
    #[serde(rename = "entitlement.usage_threshold_reached")]
    EntitlementUsageThresholdReached,
    #[strum(serialize = "entitlement.limit_reached")]
    #[serde(rename = "entitlement.limit_reached")]
    EntitlementLimitReached,
    #[strum(serialize = "budget.threshold_reached")]
    #[serde(rename = "budget.threshold_reached")]
    BudgetThresholdReached,
    #[strum(serialize = "budget.exceeded")]
    #[serde(rename = "budget.exceeded")]
    BudgetExceeded,
    #[strum(serialize = "entitlement.too_many_groups")]
    #[serde(rename = "entitlement.too_many_groups")]
    EntitlementTooManyGroups,
}

#[derive(Debug, Display, EnumIter, EnumString, Copy, Clone)]
pub enum WebhookOutEventGroupEnum {
    #[strum(serialize = "customer")]
    Customer,
    #[strum(serialize = "subscription")]
    Subscription,
    #[strum(serialize = "invoice")]
    Invoice,
    /// The accounting PDF and, when opted in, the e-invoice beside it.
    #[strum(serialize = "invoice_documents")]
    InvoiceDocuments,
    #[strum(serialize = "metric")]
    BillableMetric,
    #[strum(serialize = "quote")]
    Quote,
    #[strum(serialize = "credit_note")]
    CreditNote,
    #[strum(serialize = "plan")]
    Plan,
    #[strum(serialize = "product")]
    Product,
    #[strum(serialize = "coupon")]
    Coupon,
    #[strum(serialize = "addon")]
    AddOn,
    /// Money given back or clawed back on a settled payment.
    #[strum(serialize = "payment")]
    Payment,
    /// Credit entering or leaving a customer's wallet, and its automatic top-up.
    #[strum(serialize = "wallet")]
    Wallet,
    #[strum(serialize = "credit_grant")]
    CreditGrant,
    /// A feature entitlement's usage nearing or reaching its limit.
    #[strum(serialize = "entitlement")]
    Entitlement,
    /// A customer budget's spend nearing or reaching its amount.
    #[strum(serialize = "budget")]
    Budget,
}

impl WebhookOutEventGroupEnum {
    /// Delivered to the endpoints that subscribe, but not published in the spec while the
    /// endpoints of the same feature are beta.
    pub fn is_beta(&self) -> bool {
        matches!(
            self,
            WebhookOutEventGroupEnum::Entitlement | WebhookOutEventGroupEnum::Budget
        )
    }

    pub fn schema_name(&self) -> &'static str {
        match self {
            WebhookOutEventGroupEnum::Customer => "CustomerEvent",
            WebhookOutEventGroupEnum::Subscription => "SubscriptionEvent",
            WebhookOutEventGroupEnum::Invoice => "InvoiceEvent",
            WebhookOutEventGroupEnum::InvoiceDocuments => "InvoiceDocumentsEvent",
            WebhookOutEventGroupEnum::BillableMetric => "MetricEvent",
            WebhookOutEventGroupEnum::Quote => "QuoteEvent",
            WebhookOutEventGroupEnum::CreditNote => "CreditNoteEvent",
            WebhookOutEventGroupEnum::Plan => "PlanEvent",
            WebhookOutEventGroupEnum::Product => "ProductEvent",
            WebhookOutEventGroupEnum::Coupon => "CouponEvent",
            WebhookOutEventGroupEnum::AddOn => "AddOnEvent",
            WebhookOutEventGroupEnum::Payment => "PaymentEvent",
            WebhookOutEventGroupEnum::Wallet => "WalletFundedEvent",
            WebhookOutEventGroupEnum::CreditGrant => "CreditGrantExpiredEvent",
            WebhookOutEventGroupEnum::Entitlement => "EntitlementAlertWebhookEvent",
            WebhookOutEventGroupEnum::Budget => "BudgetAlertWebhookEvent",
        }
    }
}

impl WebhookOutEventTypeEnum {
    /// Emitted by the live usage controls (alerts, budgets, pause at zero, automatic top-ups),
    /// which only a module runs: published when the edition's modules emit it.
    pub fn needs_module(&self) -> bool {
        matches!(
            self,
            WebhookOutEventTypeEnum::WalletBalanceLow
                | WebhookOutEventTypeEnum::WalletDepleted
                | WebhookOutEventTypeEnum::WalletRecovered
                | WebhookOutEventTypeEnum::AutoTopUpFailed
                | WebhookOutEventTypeEnum::AutoTopUpSuspended
                | WebhookOutEventTypeEnum::AutoTopUpConsentChanged
                | WebhookOutEventTypeEnum::AutoTopUpCapReached
                | WebhookOutEventTypeEnum::EntitlementUsageThresholdReached
                | WebhookOutEventTypeEnum::EntitlementLimitReached
                | WebhookOutEventTypeEnum::EntitlementTooManyGroups
                | WebhookOutEventTypeEnum::BudgetThresholdReached
                | WebhookOutEventTypeEnum::BudgetExceeded
        )
    }

    pub fn group(&self) -> WebhookOutEventGroupEnum {
        match self {
            WebhookOutEventTypeEnum::CustomerCreated => WebhookOutEventGroupEnum::Customer,
            WebhookOutEventTypeEnum::SubscriptionCreated => WebhookOutEventGroupEnum::Subscription,
            WebhookOutEventTypeEnum::SubscriptionUpdated => WebhookOutEventGroupEnum::Subscription,
            WebhookOutEventTypeEnum::SubscriptionCancelled => {
                WebhookOutEventGroupEnum::Subscription
            }
            WebhookOutEventTypeEnum::SubscriptionEnded => WebhookOutEventGroupEnum::Subscription,
            WebhookOutEventTypeEnum::InvoiceCreated => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoiceFinalized => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoicePaid => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoiceVoided => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoiceClosed => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoiceConsolidated => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoiceDeleted => WebhookOutEventGroupEnum::Invoice,
            WebhookOutEventTypeEnum::InvoiceAccountingPdfGenerated => {
                WebhookOutEventGroupEnum::InvoiceDocuments
            }
            WebhookOutEventTypeEnum::BillableMetricCreated => {
                WebhookOutEventGroupEnum::BillableMetric
            }
            WebhookOutEventTypeEnum::QuoteAccepted => WebhookOutEventGroupEnum::Quote,
            WebhookOutEventTypeEnum::QuoteConverted => WebhookOutEventGroupEnum::Quote,
            WebhookOutEventTypeEnum::CreditNoteCreated => WebhookOutEventGroupEnum::CreditNote,
            WebhookOutEventTypeEnum::CreditNoteFinalized => WebhookOutEventGroupEnum::CreditNote,
            WebhookOutEventTypeEnum::CreditNoteVoided => WebhookOutEventGroupEnum::CreditNote,
            WebhookOutEventTypeEnum::PlanCreated => WebhookOutEventGroupEnum::Plan,
            WebhookOutEventTypeEnum::PlanPublished => WebhookOutEventGroupEnum::Plan,
            WebhookOutEventTypeEnum::PlanArchived => WebhookOutEventGroupEnum::Plan,
            WebhookOutEventTypeEnum::ProductCreated => WebhookOutEventGroupEnum::Product,
            WebhookOutEventTypeEnum::ProductUpdated => WebhookOutEventGroupEnum::Product,
            WebhookOutEventTypeEnum::ProductArchived => WebhookOutEventGroupEnum::Product,
            WebhookOutEventTypeEnum::BillableMetricUpdated => {
                WebhookOutEventGroupEnum::BillableMetric
            }
            WebhookOutEventTypeEnum::BillableMetricArchived => {
                WebhookOutEventGroupEnum::BillableMetric
            }
            WebhookOutEventTypeEnum::CouponCreated => WebhookOutEventGroupEnum::Coupon,
            WebhookOutEventTypeEnum::CouponUpdated => WebhookOutEventGroupEnum::Coupon,
            WebhookOutEventTypeEnum::CouponArchived => WebhookOutEventGroupEnum::Coupon,
            WebhookOutEventTypeEnum::AddOnCreated => WebhookOutEventGroupEnum::AddOn,
            WebhookOutEventTypeEnum::AddOnUpdated => WebhookOutEventGroupEnum::AddOn,
            WebhookOutEventTypeEnum::AddOnArchived => WebhookOutEventGroupEnum::AddOn,
            WebhookOutEventTypeEnum::RefundIssued => WebhookOutEventGroupEnum::Payment,
            WebhookOutEventTypeEnum::RefundSettled => WebhookOutEventGroupEnum::Payment,
            WebhookOutEventTypeEnum::RefundFailed => WebhookOutEventGroupEnum::Payment,
            WebhookOutEventTypeEnum::PaymentReversed => WebhookOutEventGroupEnum::Payment,
            WebhookOutEventTypeEnum::PaymentReinstated => WebhookOutEventGroupEnum::Payment,
            WebhookOutEventTypeEnum::PaymentFailed => WebhookOutEventGroupEnum::Payment,
            WebhookOutEventTypeEnum::WalletFunded
            | WebhookOutEventTypeEnum::WalletDebited
            | WebhookOutEventTypeEnum::WalletBalanceLow
            | WebhookOutEventTypeEnum::WalletCreditUnrecovered
            | WebhookOutEventTypeEnum::WalletCreditRecovered
            | WebhookOutEventTypeEnum::AutoTopUpFailed
            | WebhookOutEventTypeEnum::AutoTopUpSuspended
            | WebhookOutEventTypeEnum::AutoTopUpConsentChanged
            | WebhookOutEventTypeEnum::WalletDepleted
            | WebhookOutEventTypeEnum::WalletRecovered
            | WebhookOutEventTypeEnum::AutoTopUpCapReached => WebhookOutEventGroupEnum::Wallet,
            WebhookOutEventTypeEnum::CreditGrantExpired => WebhookOutEventGroupEnum::CreditGrant,
            WebhookOutEventTypeEnum::EntitlementUsageThresholdReached
            | WebhookOutEventTypeEnum::EntitlementLimitReached
            | WebhookOutEventTypeEnum::EntitlementTooManyGroups => {
                WebhookOutEventGroupEnum::Entitlement
            }
            WebhookOutEventTypeEnum::BudgetThresholdReached
            | WebhookOutEventTypeEnum::BudgetExceeded => WebhookOutEventGroupEnum::Budget,
        }
    }

    /// Payload schema of this event: one per group, except wallet events, which differ per type, and
    /// the too-many-groups events, which share one.
    pub fn schema_name(&self) -> &'static str {
        match self {
            WebhookOutEventTypeEnum::WalletDebited => "WalletDebitedEvent",
            WebhookOutEventTypeEnum::WalletBalanceLow => "WalletBalanceLowEvent",
            WebhookOutEventTypeEnum::WalletDepleted => "WalletDepletedEvent",
            WebhookOutEventTypeEnum::WalletRecovered => "WalletRecoveredEvent",
            WebhookOutEventTypeEnum::WalletCreditUnrecovered => "WalletCreditUnrecoveredEvent",
            WebhookOutEventTypeEnum::WalletCreditRecovered => "WalletCreditRecoveredEvent",
            WebhookOutEventTypeEnum::AutoTopUpFailed
            | WebhookOutEventTypeEnum::AutoTopUpSuspended => "AutoTopUpEvent",
            WebhookOutEventTypeEnum::AutoTopUpConsentChanged => "AutoTopUpConsentChangedEvent",
            WebhookOutEventTypeEnum::AutoTopUpCapReached => "AutoTopUpCapReachedEvent",
            WebhookOutEventTypeEnum::EntitlementTooManyGroups => "TooManyGroupsWebhookEvent",
            other => other.group().schema_name(),
        }
    }

    pub fn description(&self) -> String {
        match self {
            WebhookOutEventTypeEnum::CustomerCreated => "A new customer was created".to_string(),
            WebhookOutEventTypeEnum::SubscriptionCreated => {
                "A new subscription was created".to_string()
            }
            WebhookOutEventTypeEnum::SubscriptionUpdated => {
                "A subscription was updated (plan change, amendment, seats, billing configuration, ...)"
                    .to_string()
            }
            WebhookOutEventTypeEnum::SubscriptionCancelled => {
                "A subscription was cancelled (its cancellation took effect)".to_string()
            }
            WebhookOutEventTypeEnum::SubscriptionEnded => {
                "A subscription ended by reaching its fixed end date".to_string()
            }
            WebhookOutEventTypeEnum::InvoiceCreated => "A new invoice was created".to_string(),
            WebhookOutEventTypeEnum::InvoiceFinalized => "An invoice was finalized".to_string(),
            WebhookOutEventTypeEnum::InvoiceAccountingPdfGenerated => {
                "The accounting PDF of a finalized invoice was stored, with the e-invoicing outcome when the invoicing entity opted in"
                    .to_string()
            }
            WebhookOutEventTypeEnum::InvoicePaid => {
                "An invoice was paid, or credit notes settled its balance".to_string()
            }
            WebhookOutEventTypeEnum::InvoiceVoided => "An invoice was voided".to_string(),
            WebhookOutEventTypeEnum::InvoiceClosed => {
                // Terminal, non-revenue event: a recurring usage-only period settled with
                // nothing to bill (total 0, no number). The earlier `invoice.created` draft is
                // closed here and will never finalize or be paid.
                "An empty recurring invoice was closed with nothing to bill".to_string()
            }
            WebhookOutEventTypeEnum::InvoiceConsolidated => {
                // Compensating event: the referenced (child) draft invoice was already announced
                // via `invoice.created` and is now superseded by the consolidated parent
                // (`consolidated_into_invoice_id`). Integrators should treat the child as merged and
                // follow the parent for finalization, payment and PDF.
                "An invoice was merged into a consolidated invoice and is superseded by its parent"
                    .to_string()
            }
            WebhookOutEventTypeEnum::InvoiceDeleted => {
                // Only a draft can be deleted, so the referenced invoice was never issued;
                // integrators should drop it rather than expect any further event on it.
                "A draft invoice was deleted".to_string()
            }
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
            WebhookOutEventTypeEnum::PlanCreated => "A new plan was created".to_string(),
            WebhookOutEventTypeEnum::PlanPublished => "A plan version was published".to_string(),
            WebhookOutEventTypeEnum::PlanArchived => "A plan was archived".to_string(),
            WebhookOutEventTypeEnum::ProductCreated => "A new product was created".to_string(),
            WebhookOutEventTypeEnum::ProductUpdated => "A product was updated".to_string(),
            WebhookOutEventTypeEnum::ProductArchived => "A product was archived".to_string(),
            WebhookOutEventTypeEnum::BillableMetricUpdated => {
                "A billable metric was updated".to_string()
            }
            WebhookOutEventTypeEnum::BillableMetricArchived => {
                "A billable metric was archived".to_string()
            }
            WebhookOutEventTypeEnum::CouponCreated => "A new coupon was created".to_string(),
            WebhookOutEventTypeEnum::CouponUpdated => "A coupon was updated".to_string(),
            WebhookOutEventTypeEnum::CouponArchived => "A coupon was archived".to_string(),
            WebhookOutEventTypeEnum::AddOnCreated => "A new add-on was created".to_string(),
            WebhookOutEventTypeEnum::AddOnUpdated => "An add-on was updated".to_string(),
            WebhookOutEventTypeEnum::AddOnArchived => "An add-on was archived".to_string(),
            WebhookOutEventTypeEnum::RefundIssued => {
                "A refund was created and is on its way to the payment provider".to_string()
            }
            WebhookOutEventTypeEnum::RefundSettled => {
                // The invoice is untouched on purpose: the Refund credit note is what reduces it.
                "A refund was paid back to the customer; the invoice keeps its amount due"
                    .to_string()
            }
            WebhookOutEventTypeEnum::RefundFailed => {
                "The payment provider refused the refund; no money moved".to_string()
            }
            WebhookOutEventTypeEnum::PaymentReversed => {
                "A settled payment was clawed back: lost to a dispute or chargeback (the invoice stays paid) or a failed collection (the invoice reopens)"
                    .to_string()
            }
            WebhookOutEventTypeEnum::PaymentReinstated => {
                "Money clawed back from a payment came back (dispute won, chargeback cancelled)"
                    .to_string()
            }
            WebhookOutEventTypeEnum::PaymentFailed => {
                "A payment attempt was declined or failed; no money moved".to_string()
            }
            WebhookOutEventTypeEnum::WalletFunded => {
                "Credit was added to a wallet (purchase, top-up, plan credit, coupon, credit note, manual grant, or credit a voided invoice gave back)"
                    .to_string()
            }
            WebhookOutEventTypeEnum::WalletDebited => {
                "Credit left a wallet: applied to an invoice, taken back by a credit note, or removed by hand"
                    .to_string()
            }
            WebhookOutEventTypeEnum::WalletBalanceLow => {
                "A wallet's balance (credit on account net of unbilled usage) fell below one of its low-balance thresholds"
                    .to_string()
            }
            WebhookOutEventTypeEnum::WalletDepleted => {
                "A wallet's balance reached zero: its credit is used up".to_string()
            }
            WebhookOutEventTypeEnum::WalletRecovered => {
                "A wallet's balance came back above zero".to_string()
            }
            WebhookOutEventTypeEnum::AutoTopUpCapReached => {
                "A spending cap suppressed an automatic top-up".to_string()
            }
            WebhookOutEventTypeEnum::WalletCreditUnrecovered => {
                "Purchased credit was already spent when its money came back (reversed payment or refund), so part of it could not be taken back from the wallet"
                    .to_string()
            }
            WebhookOutEventTypeEnum::WalletCreditRecovered => {
                "Spent purchased credit reported as unrecovered is backed again: its money came back, or the credit was returned and taken back"
                    .to_string()
            }
            WebhookOutEventTypeEnum::AutoTopUpFailed => {
                "An automatic top-up charge failed".to_string()
            }
            WebhookOutEventTypeEnum::AutoTopUpSuspended => {
                "Automatic top-up was suspended after repeated failures".to_string()
            }
            WebhookOutEventTypeEnum::AutoTopUpConsentChanged => {
                "Automatic top-up was authorized, changed or turned off".to_string()
            }
            WebhookOutEventTypeEnum::CreditGrantExpired => {
                "A credit grant expired with value left; that value left the wallet".to_string()
            }
            WebhookOutEventTypeEnum::EntitlementUsageThresholdReached => {
                "Usage of a feature entitlement reached one of its notification thresholds"
                    .to_string()
            }
            WebhookOutEventTypeEnum::EntitlementLimitReached => {
                "Usage of a feature entitlement reached its limit".to_string()
            }
            WebhookOutEventTypeEnum::BudgetThresholdReached => {
                "A customer's spend, or a subscription's, reached one of the notification thresholds of its budget"
                    .to_string()
            }
            WebhookOutEventTypeEnum::BudgetExceeded => {
                "A customer's spend reached its budget".to_string()
            }
            WebhookOutEventTypeEnum::EntitlementTooManyGroups => {
                "A grouped feature entitlement that refuses stopped tracking new groups: callers of a group it does not keep are refused"
                    .to_string()
            }
        }
    }
}

#[cfg(test)]
mod wallet_payload_tests {
    use super::*;
    use chrono::Utc;
    use common_domain::ids::{TenantId, WalletId};
    use meteroid_store::domain::credit::GrantScope;
    use meteroid_store::domain::wallet_events::{
        WalletAutoTopUpConsentChangedEvent, WalletBalanceLowEvent, WalletDebitedEvent,
        WalletRecoveredEvent,
    };

    fn consent_actor(actor: String) -> serde_json::Value {
        let event = WalletAutoTopUpConsentChangedEvent::new(
            TenantId::new(),
            CustomerId::new(),
            WalletId::new(),
            "EUR".to_owned(),
            None,
            Some("rev".to_owned()),
            actor,
            Utc::now(),
        );
        serde_json::to_value(WebhookOutAutoTopUpConsentChangedEventData::from(event)).unwrap()
            ["actor"]
            .clone()
    }

    #[test]
    fn a_consent_change_names_the_api_key_that_attested_it() {
        let key = uuid::Uuid::now_v7();
        let at = Utc::now();
        let event = WalletAutoTopUpConsentChangedEvent::new(
            TenantId::new(),
            CustomerId::new(),
            WalletId::new(),
            "EUR".to_owned(),
            None,
            Some("rev".to_owned()),
            format!("api_token:{key}"),
            at,
        )
        .attested(Some(format!("api_token:{key}")), Some(at));
        let payload =
            serde_json::to_value(WebhookOutAutoTopUpConsentChangedEventData::from(event)).unwrap();
        let api_key = serde_json::json!({
            "type": "API_KEY",
            "id": common_domain::ids::ApiTokenId::from(key).as_base62(),
        });
        assert_eq!(payload["actor"], api_key);
        assert_eq!(payload["consent_attested_by"], api_key);
        assert!(payload["consent_attested_at"].is_string(), "{payload}");
        assert!(consent_actor("system".to_owned()).get("id").is_none());
    }

    #[test]
    fn balance_events_report_the_balance_under_the_rest_name() {
        let low = serde_json::to_value(WebhookOutWalletBalanceLowEventData::from(
            WalletBalanceLowEvent::new(
                TenantId::new(),
                CustomerId::new(),
                WalletId::new(),
                "EUR".to_owned(),
                800,
                1_000,
            )
            .with_shared(300),
        ))
        .unwrap();
        assert_eq!(low["balance"], 800);
        assert_eq!(low["threshold"], 1_000);
        assert_eq!(low["shared_balance"], 300);
        assert!(low.get("available_balance").is_none());

        let recovered = serde_json::to_value(WebhookOutWalletRecoveredEventData::from(
            WalletRecoveredEvent::new(
                TenantId::new(),
                CustomerId::new(),
                WalletId::new(),
                "EUR".to_owned(),
                500,
            ),
        ))
        .unwrap();
        assert_eq!(recovered["balance"], 500);
        assert!(recovered.get("available_balance").is_none());
    }

    #[test]
    fn a_debit_by_hand_names_its_user_as_a_typed_actor() {
        let user = UserId::new();
        let debited = |actor_id: Option<uuid::Uuid>| {
            serde_json::to_value(WebhookOutWalletDebitedEventData::from(
                WalletDebitedEvent::new(
                    TenantId::new(),
                    CustomerId::new(),
                    WalletId::new(),
                    &GrantScope::customer_wide(),
                    LedgerEntryId::new(),
                    CreditGrantId::new(),
                    None,
                    None,
                    100,
                    "EUR".to_owned(),
                    "Removed".to_owned(),
                    actor_id,
                ),
            ))
            .unwrap()
        };
        let by_hand = debited(Some(user.as_uuid()));
        assert_eq!(
            by_hand["actor"],
            serde_json::json!({ "type": "USER", "id": user.as_base62() })
        );
        assert!(by_hand.get("actor_id").is_none());
        assert!(debited(None).get("actor").is_none());
    }

    #[test]
    fn consent_changes_name_who_made_them() {
        let customer = CustomerId::new();
        assert_eq!(
            consent_actor(format!("customer:{}", customer.as_base62())),
            serde_json::json!({ "type": "CUSTOMER", "id": customer.as_base62() })
        );
        let user = UserId::new();
        assert_eq!(
            consent_actor(format!("user:{}", user.as_uuid())),
            serde_json::json!({ "type": "USER", "id": user.as_base62() })
        );
        assert_eq!(
            consent_actor("system".to_owned()),
            serde_json::json!({ "type": "SYSTEM" })
        );
    }
}
