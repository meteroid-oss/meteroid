use diesel_models::enums as diesel_enums;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use strum::{Display, EnumString};

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::BankAccountFormat)]
pub enum BankAccountFormat {
    IbanBicSwift,
    AccountRouting,
    SortCodeAccount,
    AccountBicSwift,
}

/// Core entities that can carry user-defined custom properties.
#[derive(
    o2o, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, Display, EnumString,
)]
#[map_owned(diesel_enums::CustomPropertyEntityTypeEnum)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CustomPropertyEntityType {
    Customer,
    Subscription,
    Invoice,
    CreditNote,
    Quote,
}

/// The entity lists that keep saved views.
#[derive(
    o2o, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, Display, EnumString,
)]
#[map_owned(diesel_enums::EntityListTypeEnum)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntityListType {
    Customer,
    Subscription,
    Invoice,
    CreditNote,
    Quote,
    /// The signals worklist.
    Signal,
}

/// Value type of a custom property definition; drives validation and UI rendering.
#[derive(
    o2o, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, Display, EnumString,
)]
#[map_owned(diesel_enums::CustomPropertyTypeEnum)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CustomPropertyType {
    Text,
    Number,
    Boolean,
    Date,
    Datetime,
    SingleSelect,
    MultiSelect,
    Json,
    Url,
    Email,
}

#[derive(o2o, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::BillingMetricAggregateEnum)]
pub enum BillingMetricAggregateEnum {
    Count,
    Latest,
    Max,
    Min,
    Mean,
    Sum,
    CountDistinct,
}

#[derive(o2o, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Default)]
#[map_owned(diesel_enums::BillingPeriodEnum)]
pub enum BillingPeriodEnum {
    #[default]
    Monthly,
    Quarterly,
    Semiannual,
    Annual,
}

impl BillingPeriodEnum {
    pub fn as_months(&self) -> u32 {
        match self {
            BillingPeriodEnum::Monthly => 1,
            BillingPeriodEnum::Quarterly => 3,
            BillingPeriodEnum::Semiannual => 6,
            BillingPeriodEnum::Annual => 12,
        }
    }

    pub fn as_subscription_billing_period(&self) -> SubscriptionFeeBillingPeriod {
        match self {
            BillingPeriodEnum::Monthly => SubscriptionFeeBillingPeriod::Monthly,
            BillingPeriodEnum::Quarterly => SubscriptionFeeBillingPeriod::Quarterly,
            BillingPeriodEnum::Semiannual => SubscriptionFeeBillingPeriod::Semiannual,
            BillingPeriodEnum::Annual => SubscriptionFeeBillingPeriod::Annual,
        }
    }
}

impl PartialOrd for BillingPeriodEnum {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BillingPeriodEnum {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_months().cmp(&other.as_months())
    }
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[map_owned(diesel_enums::FeeTypeEnum)]
pub enum FeeTypeEnum {
    Rate,
    Slot,
    Capacity,
    Usage,
    ExtraRecurring,
    OneTime,
    Credit,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[map_owned(diesel_enums::CreditNoteStatus)]
pub enum CreditNoteStatus {
    Draft,
    Finalized,
    Voided,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, Eq, PartialEq)]
#[map_owned(diesel_enums::CreditTypeEnum)]
pub enum CreditType {
    CreditToBalance,
    Refund,
    DebtCancellation,
}

/// Company vs. natural person (B2C). Drives first+last-name addressing and, for
/// `Individual`, consumer VAT resolution (which needs a billing-address country).
#[derive(
    o2o, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Default, Display, EnumString,
)]
#[map_owned(diesel_enums::CustomerTypeEnum)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CustomerType {
    #[default]
    Company,
    Individual,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, Eq, PartialEq)]
#[map_owned(diesel_enums::InvoiceStatusEnum)]
pub enum InvoiceStatusEnum {
    Draft,
    Finalized,
    Void,
    Uncollectible,
    Closed,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, Eq, PartialEq)]
#[map_owned(diesel_enums::InvoicePaymentStatus)]
pub enum InvoicePaymentStatus {
    Unpaid,
    PartiallyPaid,
    Paid,
    Errored,
    /// Payment accepted by the provider, not yet settled (async direct debit).
    Processing,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[map_owned(diesel_enums::InvoiceType)]
pub enum InvoiceType {
    Recurring,
    OneOff,
    Adjustment,
    // Imported,
    UsageThreshold,
    /// What a subscription cancelled at once consumed after its period's own invoice, outside
    /// the period key that invoice holds.
    Final,
}

impl InvoiceType {
    /// Priced again from the subscription's usage until issued: a period's invoice or a final one.
    pub fn bills_subscription_usage(&self) -> bool {
        matches!(self, InvoiceType::Recurring | InvoiceType::Final)
    }
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[map_owned(diesel_enums::ConnectorStatusEnum)]
pub enum ConnectorStatusEnum {
    /// Being set up; never used for payments.
    Draft,
    Active,
    /// Starts nothing new; in-flight payments still settle.
    Disabled,
    /// Soft-deleted: hidden, credentials wiped.
    Deleted,
}

/// Where a provider sends a connector's webhooks.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[map_owned(diesel_enums::ConnectorWebhookRouteEnum)]
pub enum ConnectorWebhookRouteEnum {
    /// `/webhooks/v1/payments/{connector_id}`.
    ConnectorId,
    /// `/webhooks/v1/{tenant}/{alias}`: connectors set up before connector-id URLs. The alias is
    /// part of the URL, so they can't be renamed.
    TenantAlias,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::ConnectorTypeEnum)]
pub enum ConnectorTypeEnum {
    Accounting,
    Crm,
    PaymentProvider,
    Tax,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionTypeEnum {
    Card,
    DirectDebit,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::ConnectorProviderEnum)]
pub enum ConnectorProviderEnum {
    Hubspot,
    Stripe,
    Pennylane,
    Mock,
    Gocardless,
    Stancer,
    Mollie,
}

/// The providers that can take payments: the payment subset of [`ConnectorProviderEnum`].
/// Holding one is proof that a payment adapter exists for it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum PaymentProvider {
    Stripe,
    Gocardless,
    Stancer,
    Mollie,
    Mock,
}

impl ConnectorProviderEnum {
    /// `None` for the providers that take no payments (Hubspot, Pennylane).
    pub fn as_payment(&self) -> Option<PaymentProvider> {
        match self {
            ConnectorProviderEnum::Stripe => Some(PaymentProvider::Stripe),
            ConnectorProviderEnum::Gocardless => Some(PaymentProvider::Gocardless),
            ConnectorProviderEnum::Stancer => Some(PaymentProvider::Stancer),
            ConnectorProviderEnum::Mollie => Some(PaymentProvider::Mollie),
            ConnectorProviderEnum::Mock => Some(PaymentProvider::Mock),
            ConnectorProviderEnum::Hubspot | ConnectorProviderEnum::Pennylane => None,
        }
    }
}

impl From<PaymentProvider> for ConnectorProviderEnum {
    fn from(value: PaymentProvider) -> Self {
        match value {
            PaymentProvider::Stripe => ConnectorProviderEnum::Stripe,
            PaymentProvider::Gocardless => ConnectorProviderEnum::Gocardless,
            PaymentProvider::Stancer => ConnectorProviderEnum::Stancer,
            PaymentProvider::Mollie => ConnectorProviderEnum::Mollie,
            PaymentProvider::Mock => ConnectorProviderEnum::Mock,
        }
    }
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::WebhookDeliveryStatusEnum)]
#[serde(rename_all = "snake_case")]
pub enum WebhookDeliveryStatus {
    Pending,
    InFlight,
    Succeeded,
    Failed,
    Cancelled,
}

impl WebhookDeliveryStatus {
    /// Terminal statuses are never retried by the delivery worker.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            WebhookDeliveryStatus::Succeeded
                | WebhookDeliveryStatus::Failed
                | WebhookDeliveryStatus::Cancelled
        )
    }
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::WebhookEndpointDisabledReasonEnum)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEndpointDisabledReason {
    Manual,
    AutoFailures,
    Gone,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::PaymentMethodTypeEnum)]
pub enum PaymentMethodTypeEnum {
    Card,
    Transfer,
    DirectDebitSepa,
    DirectDebitAch,
    DirectDebitBacs,
    Other,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::PaymentStatusEnum)]
pub enum PaymentStatusEnum {
    Ready,   // ready to process
    Pending, // waiting for external service
    Settled,
    Cancelled,
    Failed,
    Refunded, // settled then fully clawed back (refund, chargeback, lost dispute)
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::PaymentTypeEnum)]
pub enum PaymentTypeEnum {
    Payment,
    Refund,
    /// A customer's purchase of invoice credit, charged outside any invoice collection.
    TopUp,
    /// A purchase of invoice credit charged by an automatic top-up.
    AutoTopup,
}

/// Coarse failure category for retry policy and messaging, without leaking
/// provider-specific decline codes.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[map_owned(diesel_enums::DeclineKindEnum)]
pub enum DeclineKind {
    InsufficientFunds,
    DoNotHonor,
    CardExpired,
    AuthenticationRequired,
    MandateInactive,
    Fraud,
    ProcessingError,
    Other,
    /// The issuer or network forbids re-presenting this charge (e.g. revoked authorization).
    DoNotRetry,
    /// Not a decline: the provider never confirmed the charge and it could not safely be
    /// re-sent. No automatic retry; a late confirmation still settles it.
    Unconfirmed,
}

impl DeclineKind {
    /// Charging the same instrument again cannot succeed or is forbidden by the network;
    /// an off-session retry cannot satisfy authentication either.
    pub fn is_hard(&self) -> bool {
        matches!(
            self,
            Self::CardExpired
                | Self::Fraud
                | Self::MandateInactive
                | Self::AuthenticationRequired
                | Self::DoNotRetry
        )
    }
}

impl PaymentTypeEnum {
    /// An invoice payment or the charge of a credit purchase; a refund is not a collection.
    pub fn is_collection(&self) -> bool {
        matches!(self, Self::Payment | Self::TopUp | Self::AutoTopup)
    }
}

/// Why a settled payment was involuntarily clawed back. Carried on the Payment row; a voluntary
/// refund is a REFUND child transaction and never sets this.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[map_owned(diesel_enums::ReversalKindEnum)]
pub enum ReversalKind {
    Refund,
    Chargeback,
    DebtorRecall,
    InsufficientFunds,
    MandateInvalid,
    Returned,
    Dispute,
    /// Split out of the old single `amount_refunded` column by the refunds migration: the money
    /// moved before this model existed and nothing records whether it was voluntary.
    Migrated,
    Other,
}

impl ReversalKind {
    /// A claim by the payer (chargeback, recall under a debit guarantee, card dispute): an operator
    /// recording one keeps the invoice paid and books the loss, like a provider's dispute.
    pub fn is_dispute(&self) -> bool {
        matches!(self, Self::Chargeback | Self::DebtorRecall | Self::Dispute)
    }

    /// Whether the reopened invoice is worth collecting again — a temporary bank-side failure
    /// rather than a customer dispute. Exposed for the collection policy; nothing acts on it yet.
    pub fn re_collectable(&self) -> bool {
        matches!(self, Self::InsufficientFunds | Self::Returned | Self::Other)
    }

    /// Whether reminders restart on the reopened invoice: only for a bank-side failure the
    /// customer may not know about. A contested or unexplained claw-back waits for an operator.
    pub fn restarts_reminders(&self) -> bool {
        match self {
            Self::InsufficientFunds | Self::MandateInvalid | Self::Returned => true,
            Self::Chargeback
            | Self::DebtorRecall
            | Self::Dispute
            | Self::Refund
            | Self::Migrated
            | Self::Other => false,
        }
    }
}

/// How a voluntary refund was issued: through the provider, or recorded after a wire/cash
/// movement made outside Meteroid.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[map_owned(diesel_enums::RefundModeEnum)]
pub enum RefundMode {
    Online,
    Offline,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone)]
#[map_owned(diesel_enums::MrrMovementType)]
pub enum MrrMovementType {
    NewBusiness,
    Expansion,
    Contraction,
    Churn,
    Reactivation,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, Eq, PartialEq)]
#[map_owned(diesel_enums::OrganizationUserRole)]
pub enum OrganizationUserRole {
    Admin,
    Member,
}

#[derive(o2o, Serialize, Deserialize, Debug, Default, Clone, PartialEq, Display, EnumString)]
#[map_owned(diesel_enums::PlanStatusEnum)]
pub enum PlanStatusEnum {
    #[default]
    Draft,
    Active,
    Inactive,
    Archived,
}

#[derive(o2o, Serialize, Deserialize, Debug, Default, PartialEq, Clone, Display, EnumString)]
#[map_owned(diesel_enums::PlanTypeEnum)]
pub enum PlanTypeEnum {
    Standard,
    #[default]
    Free,
    Custom,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone)]
#[map_owned(diesel_enums::UnitConversionRoundingEnum)]
pub enum UnitConversionRoundingEnum {
    Up,
    Down,
    Nearest,
    NearestHalf,
    NearestDecile,
    None,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::SlotTransactionStatusEnum)]
pub enum SlotTransactionStatusEnum {
    Pending,
    Active,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[map_owned(diesel_enums::SubscriptionActivationConditionEnum)]
pub enum SubscriptionActivationCondition {
    #[default]
    OnStart,
    OnCheckout,
    Manual,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone)]
#[map_owned(diesel_enums::SubscriptionEventType)]
pub enum SubscriptionEventType {
    Created,
    Activated,
    Switch,
    Cancelled,
    Ended,
    Reactivated,
    Updated,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::SubscriptionFeeBillingPeriod)]
pub enum SubscriptionFeeBillingPeriod {
    OneTime,
    Monthly,
    Quarterly,
    Semiannual,
    Annual,
}

impl SubscriptionFeeBillingPeriod {
    pub fn as_months(&self) -> i32 {
        match self {
            SubscriptionFeeBillingPeriod::OneTime => i32::MAX, // month_elapsed % OneTime.as_months() will only be 0 if month_elapsed is 0
            SubscriptionFeeBillingPeriod::Monthly => 1,
            SubscriptionFeeBillingPeriod::Quarterly => 3,
            SubscriptionFeeBillingPeriod::Semiannual => 6,
            SubscriptionFeeBillingPeriod::Annual => 12,
        }
    }

    pub fn as_billing_period_opt(&self) -> Option<BillingPeriodEnum> {
        match self {
            SubscriptionFeeBillingPeriod::OneTime => None,
            SubscriptionFeeBillingPeriod::Monthly => Some(BillingPeriodEnum::Monthly),
            SubscriptionFeeBillingPeriod::Quarterly => Some(BillingPeriodEnum::Quarterly),
            SubscriptionFeeBillingPeriod::Semiannual => Some(BillingPeriodEnum::Semiannual),
            SubscriptionFeeBillingPeriod::Annual => Some(BillingPeriodEnum::Annual),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum BillingType {
    Advance,
    Arrears,
}

/// Where a workspace's customer emails go.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[map_owned(diesel_enums::EmailDeliveryModeEnum)]
pub enum EmailDeliveryMode {
    /// To the customers.
    Live,
    /// Only to the organization's members and the workspace's test recipients.
    Test,
    /// To nobody.
    Off,
}

impl EmailDeliveryMode {
    /// Production goes live; every other environment starts by emailing the team only.
    pub fn default_for(environment: &TenantEnvironmentEnum) -> Self {
        match environment {
            TenantEnvironmentEnum::Production => Self::Live,
            _ => Self::Test,
        }
    }

    /// Only a production workspace can be switched to `Live`, so staging never stands in for
    /// production. Non-production workspaces already live when this shipped keep it.
    pub fn can_go_live(environment: &TenantEnvironmentEnum) -> bool {
        *environment == TenantEnvironmentEnum::Production
    }
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::TenantEnvironmentEnum)]
pub enum TenantEnvironmentEnum {
    Production,
    Staging,
    Qa,
    Development,
    Sandbox,
    Demo,
}

impl TenantEnvironmentEnum {
    pub fn as_short_string(&self) -> String {
        match self {
            TenantEnvironmentEnum::Production => "prod".to_string(),
            TenantEnvironmentEnum::Staging => "stg".to_string(),
            TenantEnvironmentEnum::Qa => "qa".to_string(),
            TenantEnvironmentEnum::Development => "dev".to_string(),
            TenantEnvironmentEnum::Sandbox => "sand".to_string(),
            TenantEnvironmentEnum::Demo => "demo".to_string(),
        }
    }

    /// Base for a tenant's URL slug; de-duplicated per organization with a numeric suffix.
    pub fn slug_base(&self) -> &'static str {
        match self {
            TenantEnvironmentEnum::Production => "prod",
            TenantEnvironmentEnum::Staging => "staging",
            TenantEnvironmentEnum::Qa => "qa",
            TenantEnvironmentEnum::Development => "dev",
            TenantEnvironmentEnum::Sandbox => "sandbox",
            TenantEnvironmentEnum::Demo => "demo",
        }
    }
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[map_owned(diesel_enums::TenantStatusEnum)]
pub enum TenantStatusEnum {
    Active,
    Paused,
    Archived,
    MarkedForDeletion,
    PendingDeletion,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[map_owned(diesel_enums::StoredObjectKindEnum)]
pub enum StoredObjectKindEnum {
    InvoicePdf,
    InvoiceXml,
    CreditNotePdf,
    ReceiptPdf,
    ImageLogo,
    WebhookArchive,
    BatchJobInput,
    BatchJobErrorOutput,
    TaxReport,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::SubscriptionStatusEnum)]
pub enum SubscriptionStatusEnum {
    PendingActivation, // before trial
    PendingCharge,     // after billing start date, while awaiting payment
    TrialActive,
    Active,
    TrialExpired, // trial ended on paid plan without payment method
    Paused,
    Suspended, // due to non-payment
    Cancelled,
    Aborted, // cancelled before ever activating
    Completed,
    Superseded, // upgrade/downgrade
    Errored,    // failed to process after max retries
}
impl SubscriptionStatusEnum {
    pub fn as_screaming_snake_case(&self) -> String {
        match self {
            SubscriptionStatusEnum::PendingActivation => "PENDING_ACTIVATION",
            SubscriptionStatusEnum::PendingCharge => "PENDING_CHARGE",
            SubscriptionStatusEnum::TrialActive => "TRIAL_ACTIVE",
            SubscriptionStatusEnum::Active => "ACTIVE",
            SubscriptionStatusEnum::TrialExpired => "TRIAL_EXPIRED",
            SubscriptionStatusEnum::Paused => "PAUSED",
            SubscriptionStatusEnum::Suspended => "SUSPENDED",
            SubscriptionStatusEnum::Cancelled => "CANCELLED",
            SubscriptionStatusEnum::Aborted => "ABORTED",
            SubscriptionStatusEnum::Completed => "COMPLETED",
            SubscriptionStatusEnum::Superseded => "SUPERSEDED",
            SubscriptionStatusEnum::Errored => "ERRORED",
        }
        .to_string()
    }

    /// Terminal states a subscription can never leave and is never re-counted as active.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            SubscriptionStatusEnum::Cancelled
                | SubscriptionStatusEnum::Aborted
                | SubscriptionStatusEnum::Completed
                | SubscriptionStatusEnum::Superseded
                | SubscriptionStatusEnum::Errored
        )
    }

    /// True while the subscription settles its current period early instead of renewing: every
    /// status `terminate_subscription` applies — the terminal states plus `Paused`. Such a period
    /// has elapsed, so an empty arrears invoice for it is still a valid finalization anchor.
    /// Keep in sync with the states passed to `terminate_subscription`.
    ///
    /// `Paused` is the one non-terminal state here, and it is only safe while there is no resume
    /// path: a Closed anchor is terminal and `invoice_subscription_recurring_period_unique` admits
    /// it, so resuming onto the same date could not re-bill that period. Revisit when resume lands.
    pub fn settles_period_early(&self) -> bool {
        self.is_terminal() || matches!(self, SubscriptionStatusEnum::Paused)
    }
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::ScheduledEventTypeEnum)]
pub enum ScheduledEventTypeEnum {
    FinalizeInvoice,
    RetryPayment,
    ApplyPlanChange,
    CancelSubscription,
    PauseSubscription,
    EndTrial,
    ApplyAmendment,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::ScheduledEventStatus)]
pub enum ScheduledEventStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Canceled,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[map_owned(diesel_enums::TaxResolverEnum)]
pub enum TaxResolverEnum {
    None,
    Manual,
    #[default]
    MeteroidEuVat,
    External,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, Eq, PartialEq)]
#[map_owned(diesel_enums::QuoteStatusEnum)]
pub enum QuoteStatusEnum {
    Draft,
    Pending,
    Accepted,
    Declined,
    Expired,
    Cancelled,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::BatchJobTypeEnum)]
pub enum BatchJobTypeEnum {
    EventCsvImport,
    CustomerCsvImport,
    SubscriptionCsvImport,
    SubscriptionPlanMigration,
    TaxReportExport,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::BatchJobStatusEnum)]
pub enum BatchJobStatusEnum {
    Pending,
    Chunking,
    Processing,
    Completed,
    CompletedWithErrors,
    Failed,
    Cancelled,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::BatchJobChunkStatusEnum)]
pub enum BatchJobChunkStatusEnum {
    Pending,
    Processing,
    Completed,
    Failed,
    Skipped,
}

/// Internal-only entitlement composition mode. Resolved automatically on the BE based on
/// the entity that owns the entitlement (e.g. add-on with `max_instances_per_subscription`
/// forces `Stack` mode). Not exposed via API/FE.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::EntitlementModeEnum)]
pub enum EntitlementModeEnum {
    Override,
    Stack,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Display, EnumString)]
#[map_owned(diesel_enums::FeatureStatusEnum)]
pub enum FeatureStatusEnum {
    Active,
    Disabled,
    Archived,
}

/// Claim-before-send state of a customer email.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::EmailSendClaimStateEnum)]
pub enum EmailSendClaimState {
    Sending,
    Sent,
    Failed,
    Unknown,
}

/// `Retry` / `RetryAndNotify` are reserved for the dunning slice.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[map_owned(diesel_enums::PaymentReminderStepActionEnum)]
pub enum PaymentReminderStepAction {
    #[default]
    Notify,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::ReminderEnrolledViaEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReminderEnrolledVia {
    CustomerOverride,
    Audience,
    Default,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::ReminderStateEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReminderState {
    NotEnrolled,
    Enrolled,
    Ended,
}

/// The only stamped negative resolution; permanent.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::ReminderNotEnrolledReasonEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReminderNotEnrolledReason {
    LadderStale,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::ReminderStopReasonEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReminderStopReason {
    Operator,
    CustomerOptOut,
    SequenceArchived,
    PaymentDisputed,
}

#[cfg(test)]
mod tests {
    use super::SubscriptionStatusEnum as S;

    /// Only a bank-side failure chases the customer again; a payer who contested the charge
    /// (card dispute, direct-debit recall) is left to an operator.
    #[test]
    fn only_bank_side_reversals_restart_reminders() {
        use super::ReversalKind as K;
        for kind in [K::InsufficientFunds, K::MandateInvalid, K::Returned] {
            assert!(kind.restarts_reminders(), "{kind:?}");
        }
        for kind in [
            K::Chargeback,
            K::DebtorRecall,
            K::Dispute,
            K::Refund,
            K::Migrated,
            K::Other,
        ] {
            assert!(!kind.restarts_reminders(), "{kind:?}");
        }
    }

    /// Only the payer's claim keeps an operator-recorded reversal's invoice paid.
    #[test]
    fn only_the_payers_claims_are_disputes() {
        use super::ReversalKind as K;
        for kind in [K::Chargeback, K::DebtorRecall, K::Dispute] {
            assert!(kind.is_dispute(), "{kind:?}");
        }
        for kind in [
            K::InsufficientFunds,
            K::MandateInvalid,
            K::Returned,
            K::Refund,
            K::Migrated,
            K::Other,
        ] {
            assert!(!kind.is_dispute(), "{kind:?}");
        }
    }

    /// Must cover every status `terminate_subscription` applies — terminal states AND `Paused` —
    /// or a paused/cancelled first period silently loses late usage. Guards against a future
    /// "simplification" back to `is_terminal()` alone.
    #[test]
    fn settles_period_early_covers_terminal_and_paused() {
        for status in [
            S::Cancelled,
            S::Completed,
            S::Aborted,
            S::Superseded,
            S::Errored,
            S::Paused,
        ] {
            assert!(
                status.settles_period_early(),
                "{status:?} ends its current period early and must anchor an empty arrears period"
            );
        }

        for status in [
            S::PendingActivation,
            S::PendingCharge,
            S::TrialActive,
            S::Active,
            S::TrialExpired,
        ] {
            assert!(
                !status.settles_period_early(),
                "{status:?} keeps cycling, so an empty cycle-0 invoice is genuinely nothing to bill"
            );
        }
    }
}
