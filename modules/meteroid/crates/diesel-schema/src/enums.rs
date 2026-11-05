#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::ActorTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ActorTypeEnum {
    System,
    User,
    ApiToken,
    Customer,
    QuoteRecipient,
    #[db_rename = "OAUTH_APP"]
    OAuthApp,
}

/// Discriminator for the stateless, invoice-time *adjustment* family (floor/cap terms).
/// `Minimum` is the only kind shipped today; `Maximum` (a per-period cap) and any future
/// floor/cap-style term slot in here as a new variant + a synthesis branch, with no new
/// tables. Stateful term/drawdown commitments are NOT modeled here (see ADR 0001).
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::AdjustmentKindEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum AdjustmentKindEnum {
    Minimum,
}

/// What changed for the live-credit evaluator (`credit_input_change`).
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::CreditInputChangeKindEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CreditInputChangeKind {
    /// A pricing or reaction input of the customer.
    Inputs,
    /// A tenant-wide input; the row has no customer.
    Tenant,
    /// The customer's ledger.
    Ledger,
    /// The customer's top-up runtime state (retry schedule, suspension, cooldown).
    TopUpRuntime,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceAnnexKindEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoiceAnnexKindEnum {
    EventTable,
    StaticText,
}

/// Core entities that can carry user-defined custom properties.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::CustomPropertyEntityTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CustomPropertyEntityTypeEnum {
    Customer,
    Subscription,
    Invoice,
    CreditNote,
    Quote,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::PortalResourceKindEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum PortalResourceKindEnum {
    Invoice,
    Quote,
    Customer,
    CheckoutSession,
    Dashboard,
}

/// The entity lists that keep saved views.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::EntityListTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum EntityListTypeEnum {
    Customer,
    Subscription,
    Invoice,
    CreditNote,
    Quote,
    Signal,
}

/// Value type of a custom property definition; drives validation and UI rendering.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::CustomPropertyTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CustomPropertyTypeEnum {
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

impl From<common_domain::actor::ActorType> for ActorTypeEnum {
    fn from(a: common_domain::actor::ActorType) -> Self {
        match a {
            common_domain::actor::ActorType::System => Self::System,
            common_domain::actor::ActorType::User => Self::User,
            common_domain::actor::ActorType::ApiToken => Self::ApiToken,
            common_domain::actor::ActorType::Customer => Self::Customer,
            common_domain::actor::ActorType::QuoteRecipient => Self::QuoteRecipient,
            common_domain::actor::ActorType::OAuthApp => Self::OAuthApp,
        }
    }
}

impl From<ActorTypeEnum> for common_domain::actor::ActorType {
    fn from(a: ActorTypeEnum) -> Self {
        match a {
            ActorTypeEnum::System => Self::System,
            ActorTypeEnum::User => Self::User,
            ActorTypeEnum::ApiToken => Self::ApiToken,
            ActorTypeEnum::Customer => Self::Customer,
            ActorTypeEnum::QuoteRecipient => Self::QuoteRecipient,
            ActorTypeEnum::OAuthApp => Self::OAuthApp,
        }
    }
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::BankAccountFormat"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum BankAccountFormat {
    IbanBicSwift,
    AccountRouting,
    SortCodeAccount,
    AccountBicSwift,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::BillingMetricAggregateEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum BillingMetricAggregateEnum {
    Count,
    Latest,
    Max,
    Min,
    Mean,
    Sum,
    CountDistinct,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::BillingPeriodEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum BillingPeriodEnum {
    Monthly,
    Quarterly,
    Semiannual,
    Annual,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::FeeTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum FeeTypeEnum {
    Rate,
    Slot,
    Capacity,
    Usage,
    ExtraRecurring,
    OneTime,
    Credit,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::CreditNoteStatus"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CreditNoteStatus {
    Draft,
    Finalized,
    Voided,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::CreditTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CreditTypeEnum {
    CreditToBalance,
    Refund,
    DebtCancellation,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoiceStatusEnum {
    Draft,
    Finalized,
    Void,
    Uncollectible, // manual status. Use if the invoice will not be paid, e.g. customer is bankrupt
    Closed, // draft settled with nothing to bill (e.g. usage-only period with no usage). Never numbered.
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::QuoteStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum QuoteStatusEnum {
    Draft,
    Pending,
    Accepted,
    Declined,
    Expired,
    Cancelled,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::InvoicePaymentStatus"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoicePaymentStatus {
    Unpaid,
    PartiallyPaid,
    Paid,
    Errored,
    /// A payment has been accepted by the provider but has not settled yet
    /// (delayed-notification rails: SEPA/ACH/BACS direct debit). Money is moving;
    /// the invoice must not be re-charged nor chased by dunning.
    Processing,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceType"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoiceType {
    Recurring,
    OneOff,
    Adjustment,
    // Imported,
    UsageThreshold,
    /// What a subscription cancelled at once consumed after its period's own invoice.
    Final,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::ConnectorProviderEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConnectorProviderEnum {
    Stripe,
    Hubspot,
    Pennylane,
    Mock,
    Gocardless,
    Stancer,
    Mollie,
}

impl ConnectorProviderEnum {
    pub fn as_meta_key(&self) -> &str {
        match self {
            ConnectorProviderEnum::Stripe => "stripe",
            ConnectorProviderEnum::Hubspot => "hubspot",
            ConnectorProviderEnum::Pennylane => "pennylane",
            ConnectorProviderEnum::Mock => "mock",
            ConnectorProviderEnum::Gocardless => "gocardless",
            ConnectorProviderEnum::Stancer => "stancer",
            ConnectorProviderEnum::Mollie => "mollie",
        }
    }
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::ConnectorStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConnectorStatusEnum {
    Draft,
    Active,
    Disabled,
    Deleted,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::ConnectorWebhookRouteEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConnectorWebhookRouteEnum {
    ConnectorId,
    TenantAlias,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::ConnectorTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConnectorTypeEnum {
    PaymentProvider,
    Crm,
    Accounting,
    Tax,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::MrrMovementType"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum MrrMovementType {
    NewBusiness,
    Expansion,
    Contraction,
    Churn,
    Reactivation,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::OrganizationUserRole"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum OrganizationUserRole {
    Admin,
    Member,
}

impl std::fmt::Display for OrganizationUserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrganizationUserRole::Admin => write!(f, "Owner"),
            OrganizationUserRole::Member => write!(f, "Member"),
        }
    }
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::PaymentMethodTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum PaymentMethodTypeEnum {
    Card,
    Transfer,
    DirectDebitSepa,
    DirectDebitAch,
    DirectDebitBacs,
    Other,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::PaymentStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum PaymentStatusEnum {
    Ready,
    Pending,
    Settled,
    Cancelled,
    Failed,
    /// Settled then fully clawed back (refund, chargeback, or lost dispute).
    Refunded,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::PaymentTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum PaymentTypeEnum {
    Payment,
    Refund,
    /// A customer-initiated purchase of invoice credit, charged outside any invoice collection.
    TopUp,
    /// A purchase of invoice credit charged by the account's automatic top-up rule.
    AutoTopup,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::DeclineKindEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum DeclineKindEnum {
    InsufficientFunds,
    DoNotHonor,
    CardExpired,
    AuthenticationRequired,
    MandateInactive,
    Fraud,
    ProcessingError,
    Other,
    DoNotRetry,
    Unconfirmed,
}

impl PaymentTypeEnum {
    /// Money collected from the customer against a document: an invoice payment or the charge
    /// of a credit purchase. A refund moves money the other way.
    pub const COLLECTIONS: [PaymentTypeEnum; 3] = [Self::Payment, Self::TopUp, Self::AutoTopup];

    pub fn is_collection(self) -> bool {
        Self::COLLECTIONS.contains(&self)
    }
}

/// Why a settled payment was involuntarily clawed back. Set on the Payment row, never on a
/// REFUND child row (a voluntary refund is not a reversal).
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::ReversalKindEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ReversalKindEnum {
    Refund,
    Chargeback,
    DebtorRecall,
    InsufficientFunds,
    MandateInvalid,
    Returned,
    Dispute,
    /// Written only by the backfill that split `amount_refunded` in two: the money moved before
    /// this model existed and nothing records whether it was voluntary.
    Migrated,
    Other,
}

/// How a voluntary refund was issued: through the provider, or recorded after a wire/cash
/// movement made outside Meteroid.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::RefundModeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum RefundModeEnum {
    Online,
    Offline,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Default)]
#[ExistingTypePath = "crate::schema::sql_types::PlanStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum PlanStatusEnum {
    #[default]
    Draft,
    Active,
    Inactive,
    Archived,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Default, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::PlanTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum PlanTypeEnum {
    Standard,
    #[default]
    Free,
    Custom,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Default, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::SubscriptionActivationConditionEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum SubscriptionActivationConditionEnum {
    OnStart,
    OnCheckout,
    #[default]
    Manual,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::SubscriptionFeeBillingPeriod"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum SubscriptionFeeBillingPeriod {
    OneTime,
    Monthly,
    Quarterly,
    Semiannual,
    Annual,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::SubscriptionEventType"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum SubscriptionEventType {
    Created,
    Activated,
    Switch,
    Cancelled,
    Ended,
    Reactivated,
    Updated,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::EmailDeliveryModeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum EmailDeliveryModeEnum {
    Live,
    Test,
    Off,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::TenantEnvironmentEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum TenantEnvironmentEnum {
    Production,
    Staging,
    Qa,
    Development,
    Sandbox,
    Demo,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::SandboxPoolStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum SandboxPoolStatusEnum {
    Warming,
    Ready,
    Claimed,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::SandboxSeedStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum SandboxSeedStatusEnum {
    InProgress,
    Completed,
    Failed,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::TenantStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum TenantStatusEnum {
    Active,
    Paused,
    Archived,
    MarkedForDeletion,
    PendingDeletion,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::UnitConversionRoundingEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum UnitConversionRoundingEnum {
    Up,
    Down,
    Nearest,
    NearestHalf,
    NearestDecile,
    None,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::SubscriptionStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
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
    pub fn not_terminal() -> Vec<SubscriptionStatusEnum> {
        vec![
            SubscriptionStatusEnum::PendingActivation,
            SubscriptionStatusEnum::PendingCharge,
            SubscriptionStatusEnum::TrialActive,
            SubscriptionStatusEnum::Active,
            SubscriptionStatusEnum::Paused,
        ]
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
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::CycleActionEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CycleActionEnum {
    // GenerateInvoice,
    ActivateSubscription,
    RenewSubscription,
    EndTrial,
    EndSubscription,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::ScheduledEventTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ScheduledEventTypeEnum {
    FinalizeInvoice,
    RetryPayment,
    ApplyPlanChange,
    CancelSubscription,
    PauseSubscription,
    EndTrial,
    ApplyAmendment,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::ScheduledEventStatus"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ScheduledEventStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Canceled,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::SlotTransactionStatus"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum SlotTransactionStatusEnum {
    Pending,
    Active,
}

/// Whether the structured e-invoice was produced for a document.
///
/// `None` means the entity had not opted in when the document was issued —
/// distinct from `Failed`, which means it opted in and the document could not
/// satisfy the standard.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::EInvoicingStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum EInvoicingStatusEnum {
    Generated,
    Failed,
}

/// The structured e-invoicing standard an invoicing entity issues under.
///
/// Siblings, not levels: XRechnung and Peppol BIS are each a CIUS of EN 16931
/// that forbids what the other allows, and Factur-X is the core model carried
/// inside the PDF rather than beside it.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::EInvoicingFormatEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum EInvoicingFormatEnum {
    FacturX,
    #[db_rename = "XRECHNUNG"]
    XRechnung,
    PeppolBis,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::TaxResolverEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum TaxResolverEnum {
    None,
    Manual,
    MeteroidEuVat,
    External,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[ExistingTypePath = "crate::schema::sql_types::CustomerTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CustomerTypeEnum {
    #[default]
    Company,
    Individual,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::TaxIdCollectionEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum TaxIdCollectionEnum {
    Off,
    Optional,
    Required,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::CustomerVatValidationStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CustomerVatValidationStatusEnum {
    Pending,
    Valid,
    Invalid,
    Unavailable,
}

/// Tri-state tax treatment of a customer (party status).
/// Taxable: normal taxation. Exempt: no tax (e.g. charity, treaty exemption).
/// ReverseCharge: tax accounted for by the buyer (B2B intra-EU), additive to the
/// VIES-derived reverse charge the engine already computes.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[ExistingTypePath = "crate::schema::sql_types::CustomerTaxStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CustomerTaxStatusEnum {
    #[default]
    Taxable,
    Exempt,
    ReverseCharge,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::CheckoutSessionStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CheckoutSessionStatusEnum {
    Created,
    AwaitingPayment,
    Completed,
    Expired,
    Cancelled,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq, Default)]
#[ExistingTypePath = "crate::schema::sql_types::CheckoutTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CheckoutTypeEnum {
    TopUp,
    #[default]
    SelfServe,
    SubscriptionActivation,
    PlanChange,
    AddonPurchase,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::ConnectConnectionTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConnectConnectionTypeEnum {
    Standard,
    Express,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::ConnectConnectionStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConnectConnectionStatusEnum {
    Pending,
    Active,
    Revoked,
    Suspended,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::ConnectOnboardingModeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConnectOnboardingModeEnum {
    Express,
    Full,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::BatchJobTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum BatchJobTypeEnum {
    EventCsvImport,
    CustomerCsvImport,
    SubscriptionCsvImport,
    SubscriptionPlanMigration,
    TaxReportExport,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::BatchJobStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum BatchJobStatusEnum {
    Pending,
    Chunking,
    Processing,
    Completed,
    CompletedWithErrors,
    Failed,
    Cancelled,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::DeadLetterStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum DeadLetterStatusEnum {
    Pending,
    Requeued,
    Discarded,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::BatchJobChunkStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum BatchJobChunkStatusEnum {
    Pending,
    Processing,
    Completed,
    Failed,
    Skipped,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::FeatureTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum FeatureTypeEnum {
    Boolean,
    Metered,
    Config,
}

/// The authoritative value type of a Config feature. `Select` additionally constrains the value
/// to `feature.config_options`; `Map`/`Json` both store a JSON `ConfigValue` (Map = key/value editor).
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::ConfigValueTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConfigValueTypeEnum {
    Number,
    Boolean,
    Text,
    Map,
    Json,
    Select,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::FeatureStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum FeatureStatusEnum {
    Active,
    Disabled,
    Archived,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::EntitlementModeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum EntitlementModeEnum {
    Override,
    Stack,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[ExistingTypePath = "crate::schema::sql_types::EntitlementEntityTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum EntitlementEntityTypeEnum {
    Feature,
    Customer,
    PlanVersion,
    AddOn,
    Plan,
    Subscription,
    Quote,
}

impl From<&common_domain::ids::EntitlementEntityId> for EntitlementEntityTypeEnum {
    fn from(value: &common_domain::ids::EntitlementEntityId) -> Self {
        match value {
            common_domain::ids::EntitlementEntityId::Feature(_) => {
                EntitlementEntityTypeEnum::Feature
            }
            common_domain::ids::EntitlementEntityId::Customer(_) => {
                EntitlementEntityTypeEnum::Customer
            }
            common_domain::ids::EntitlementEntityId::PlanVersion(_) => {
                EntitlementEntityTypeEnum::PlanVersion
            }
            common_domain::ids::EntitlementEntityId::AddOn(_) => EntitlementEntityTypeEnum::AddOn,
            common_domain::ids::EntitlementEntityId::Plan(_) => EntitlementEntityTypeEnum::Plan,
            common_domain::ids::EntitlementEntityId::Subscription(_) => {
                EntitlementEntityTypeEnum::Subscription
            }
            common_domain::ids::EntitlementEntityId::Quote(_) => EntitlementEntityTypeEnum::Quote,
        }
    }
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::AlertConditionKindEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum AlertConditionKindEnum {
    Threshold,
    PctChange,
    Anomaly,
    Limit,
    Forecast,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::AlertSeverityEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum AlertSeverityEnum {
    Info,
    Warning,
    Critical,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::MetricTargetPeriodGrainEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum MetricTargetPeriodGrainEnum {
    Month,
    Quarter,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::MetricTargetChangeKindEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum MetricTargetChangeKindEnum {
    Created,
    Updated,
    Deleted,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::ChannelTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ChannelTypeEnum {
    Slack,
    Email,
    Webhook,
    Discord,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::AlertEventStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum AlertEventStatusEnum {
    Firing,
    Acknowledged,
    Resolved,
}

/// The kind of stored document, i.e. which object-store prefix/type it belongs to.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::StoredObjectKindEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
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

/// Canonical activation milestones (dogfooding analytics): the single source
/// of truth for derivation, the funnel API and filters. `as_str` values feed
/// stable event ids; `FUNNEL` is the display order.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::MilestoneEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum MilestoneEnum {
    FirstCustomerCreated,
    FirstBillableMetricCreated,
    FirstPlanCreated,
    FirstPlanPublished,
    FirstApiKeyCreated,
    FirstApiKeyUsed,
    FirstEventIngested,
    FirstSubscriptionCreated,
    FirstInvoiceFinalized,
    FirstAddonCreated,
    FirstFeatureCreated,
    FirstQuoteCreated,
    FirstCouponCreated,
    FirstCreditNoteCreated,
    FirstStripeConnected,
    FirstGocardlessConnected,
}

/// What a *customer* of a tenant did on the way to first revenue. Distinct from
/// [`MilestoneEnum`], which tracks an *organization* configuring Meteroid.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::CustomerMilestoneEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CustomerMilestoneEnum {
    CustomerCreated,
    QuoteCreated,
    QuoteAccepted,
    CheckoutCompleted,
    TrialStarted,
    SubscriptionActivated,
    PaymentMethodAttached,
    SeatsProvisioned,
    FirstInvoiceFinalized,
    FirstInvoicePaid,
    FirstEventIngested,
}

impl CustomerMilestoneEnum {
    /// For asserting exhaustiveness where the compiler cannot: SQL labels.
    pub const ALL: &'static [CustomerMilestoneEnum] = &[
        CustomerMilestoneEnum::CustomerCreated,
        CustomerMilestoneEnum::QuoteCreated,
        CustomerMilestoneEnum::QuoteAccepted,
        CustomerMilestoneEnum::CheckoutCompleted,
        CustomerMilestoneEnum::TrialStarted,
        CustomerMilestoneEnum::SubscriptionActivated,
        CustomerMilestoneEnum::PaymentMethodAttached,
        CustomerMilestoneEnum::SeatsProvisioned,
        CustomerMilestoneEnum::FirstInvoiceFinalized,
        CustomerMilestoneEnum::FirstInvoicePaid,
        CustomerMilestoneEnum::FirstEventIngested,
    ];

    /// The Postgres label, in DbValueStyle's SCREAMING_SNAKE_CASE.
    pub fn as_label(self) -> String {
        format!("{self:?}")
            .chars()
            .flat_map(|c| {
                if c.is_uppercase() {
                    vec!['_', c]
                } else {
                    vec![c.to_ascii_uppercase()]
                }
            })
            .collect::<String>()
            .trim_start_matches('_')
            .to_string()
    }
}

/// A viewer-chosen lens over the same milestones: the steps differ per fee
/// type, so a hybrid plan's customer reads under more than one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationTemplate {
    /// Usage-priced plans, self-serve: land, integrate, go live, get paid.
    Usage,
    /// Seat-priced plans: seats bought becoming seats provisioned.
    Seats,
    /// Flat recurring, self-serve: trial or checkout straight to a card.
    SelfServe,
    /// Flat recurring, sales-led: quote to cash.
    Contract,
}

impl CustomerMilestoneEnum {
    /// Whether missing this step stops the walk. The spine — created, quoted,
    /// subscribed, invoiced, paid — is genuinely sequential: no invoice without
    /// a subscription, no payment without an invoice. The rest are setup facts
    /// that gate nothing, so a customer who skipped one but was invoiced and
    /// paid must still be counted at those steps.
    pub fn blocks_activation(self) -> bool {
        match self {
            CustomerMilestoneEnum::CustomerCreated
            | CustomerMilestoneEnum::QuoteCreated
            | CustomerMilestoneEnum::QuoteAccepted
            // A subscription that activates through checkout cannot activate
            // without one, and eligibility already limits the step to those.
            | CustomerMilestoneEnum::CheckoutCompleted
            | CustomerMilestoneEnum::SubscriptionActivated
            | CustomerMilestoneEnum::FirstInvoiceFinalized
            | CustomerMilestoneEnum::FirstInvoicePaid => true,
            CustomerMilestoneEnum::TrialStarted
            | CustomerMilestoneEnum::SeatsProvisioned
            | CustomerMilestoneEnum::PaymentMethodAttached
            | CustomerMilestoneEnum::FirstEventIngested => false,
        }
    }
}

impl ActivationTemplate {
    /// For asserting exhaustiveness where the compiler cannot: proto mapping,
    /// the default-scoring table.
    pub const ALL: &'static [ActivationTemplate] = &[
        ActivationTemplate::Usage,
        ActivationTemplate::Seats,
        ActivationTemplate::SelfServe,
        ActivationTemplate::Contract,
    ];

    /// Ordered steps, each something the customer did. The last is the first
    /// revenue outcome; nothing after it belongs to activation.
    pub fn steps(self) -> &'static [CustomerMilestoneEnum] {
        use CustomerMilestoneEnum::*;
        match self {
            ActivationTemplate::Usage => &[
                CustomerCreated,
                CheckoutCompleted,
                SubscriptionActivated,
                FirstEventIngested,
                FirstInvoiceFinalized,
                FirstInvoicePaid,
            ],
            ActivationTemplate::Seats => &[
                CustomerCreated,
                SubscriptionActivated,
                SeatsProvisioned,
                PaymentMethodAttached,
                FirstInvoiceFinalized,
                FirstInvoicePaid,
            ],
            ActivationTemplate::SelfServe => &[
                CustomerCreated,
                TrialStarted,
                CheckoutCompleted,
                SubscriptionActivated,
                PaymentMethodAttached,
                FirstInvoiceFinalized,
                FirstInvoicePaid,
            ],
            ActivationTemplate::Contract => &[
                QuoteCreated,
                QuoteAccepted,
                SubscriptionActivated,
                PaymentMethodAttached,
                FirstInvoiceFinalized,
                FirstInvoicePaid,
            ],
        }
    }

    /// The cohort anchor: the milestone the clock runs from.
    pub fn anchor(self) -> CustomerMilestoneEnum {
        self.steps()[0]
    }

    /// The lens a plan charging this way is read under.
    pub fn for_fee_type(fee: &FeeTypeEnum) -> Self {
        match fee {
            FeeTypeEnum::Slot => ActivationTemplate::Seats,
            // A prepaid credit pack is consumed by usage, so its customers walk the usage journey.
            FeeTypeEnum::Usage | FeeTypeEnum::Capacity | FeeTypeEnum::Credit => {
                ActivationTemplate::Usage
            }
            // A flat subscription is the self-serve default; add-ons and
            // one-offs ride on a deal, so they read as sales-led.
            FeeTypeEnum::Rate => ActivationTemplate::SelfServe,
            FeeTypeEnum::ExtraRecurring | FeeTypeEnum::OneTime => ActivationTemplate::Contract,
        }
    }

    /// Whichever template most of a tenant's price components map to; ties and
    /// an empty mix fall to `Usage`. A default only — the viewer picks freely.
    pub fn dominant(mix: impl IntoIterator<Item = (FeeTypeEnum, i64)>) -> Self {
        let mut scores: Vec<(ActivationTemplate, i64)> =
            ActivationTemplate::ALL.iter().map(|t| (*t, 0)).collect();
        for (fee, count) in mix {
            let template = ActivationTemplate::for_fee_type(&fee);
            if let Some(slot) = scores.iter_mut().find(|(t, _)| *t == template) {
                slot.1 += count;
            }
        }
        // Strictly-greater keeps the first entry on a tie, so Usage wins one.
        let mut best = (ActivationTemplate::Usage, 0);
        for (template, count) in scores {
            if count > best.1 {
                best = (template, count);
            }
        }
        best.0
    }
}

impl MilestoneEnum {
    /// Core journey first, then secondary feature-adoption milestones.
    pub const FUNNEL: &'static [MilestoneEnum] = &[
        MilestoneEnum::FirstCustomerCreated,
        MilestoneEnum::FirstBillableMetricCreated,
        MilestoneEnum::FirstPlanCreated,
        MilestoneEnum::FirstPlanPublished,
        MilestoneEnum::FirstApiKeyCreated,
        MilestoneEnum::FirstApiKeyUsed,
        MilestoneEnum::FirstEventIngested,
        MilestoneEnum::FirstSubscriptionCreated,
        MilestoneEnum::FirstInvoiceFinalized,
        MilestoneEnum::FirstAddonCreated,
        MilestoneEnum::FirstFeatureCreated,
        MilestoneEnum::FirstQuoteCreated,
        MilestoneEnum::FirstCouponCreated,
        MilestoneEnum::FirstCreditNoteCreated,
        MilestoneEnum::FirstStripeConnected,
        MilestoneEnum::FirstGocardlessConnected,
    ];
}

/// `Exclude` = everyone but the audience gets the kind; `Only` = only the audience does.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::EmailKindAudienceModeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum EmailKindAudienceModeEnum {
    Exclude,
    Only,
}

/// Claim-before-send state of a customer email: `Sending` = in flight, `Sent` = done,
/// `Failed` = released and re-claimable, `Unknown` = stale claim, terminal.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::EmailSendClaimStateEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum EmailSendClaimStateEnum {
    Sending,
    Sent,
    Failed,
    Unknown,
}

/// `Retry` / `RetryAndNotify` are reserved for the dunning slice.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::PaymentReminderStepActionEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum PaymentReminderStepActionEnum {
    Notify,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::ReminderEnrolledViaEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ReminderEnrolledViaEnum {
    CustomerOverride,
    Audience,
    Default,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::ReminderStateEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ReminderStateEnum {
    NotEnrolled,
    Enrolled,
    Ended,
}

/// The only stamped negative resolution; permanent.
#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::ReminderNotEnrolledReasonEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ReminderNotEnrolledReasonEnum {
    LadderStale,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::ReminderStopReasonEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ReminderStopReasonEnum {
    Operator,
    CustomerOptOut,
    SequenceArchived,
    PaymentDisputed,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::BudgetBehaviorEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum BudgetBehaviorEnum {
    Soft,
    Hard,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[ExistingTypePath = "crate::schema::sql_types::BudgetAuthorEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum BudgetAuthorEnum {
    Tenant,
    Customer,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::WebhookDeliveryStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum WebhookDeliveryStatusEnum {
    Pending,
    InFlight,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[ExistingTypePath = "crate::schema::sql_types::WebhookEndpointDisabledReasonEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum WebhookEndpointDisabledReasonEnum {
    Manual,
    AutoFailures,
    Gone,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[ExistingTypePath = "crate::schema::sql_types::CreditExhaustedRuleEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CreditExhaustedRuleEnum {
    #[default]
    Invoice,
    Pause,
    InvoiceUpTo,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[ExistingTypePath = "crate::schema::sql_types::UsageAtZeroEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum UsageAtZeroEnum {
    #[default]
    FollowPlan,
    AlwaysPause,
    NeverPause,
}
