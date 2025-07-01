#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::ActionAfterTrialEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ActionAfterTrialEnum {
    Block,
    Charge,
    Downgrade,
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
    Annual,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::CreditNoteStatus"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum CreditNoteStatus {
    Draft,
    Finalized,
    Voided,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::FangTaskState"]
#[DbValueStyle = "snake_case"]
pub enum FangTaskState {
    New,
    InProgress,
    Failed,
    Finished,
    Retried,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoiceStatusEnum {
    Draft,
    Finalized,
    Void,
    Uncollectible, // manual status. Use if the invoice will not be paid, e.g. customer is bankrupt
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone, Eq, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::InvoicePaymentStatus"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoicePaymentStatus {
    Unpaid,
    PartiallyPaid,
    Paid,
    Errored,
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
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::ConnectorProviderEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConnectorProviderEnum {
    Stripe,
    Hubspot,
    Pennylane,
}

impl ConnectorProviderEnum {
    pub fn as_meta_key(&self) -> &str {
        match self {
            ConnectorProviderEnum::Stripe => "stripe",
            ConnectorProviderEnum::Hubspot => "hubspot",
            ConnectorProviderEnum::Pennylane => "pennylane",
        }
    }
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::ConnectorTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ConnectorTypeEnum {
    PaymentProvider,
    Crm,
    Accounting,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::MrrMovementType"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum MrrMovementType {
    NewBusiness,
    Expansion,
    Contraction,
    Churn,
    Reactivation,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::OrganizationUserRole"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum OrganizationUserRole {
    Admin,
    Member,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
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
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::PaymentTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum PaymentTypeEnum {
    Payment,
    Refund,
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
    Reactivated,
    Updated,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
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
    // Planned,
    PendingActivation, // before trial
    PendingCharge,     // after billing start date, while awaiting payment
    TrialActive,
    Active,
    // PastDue,
    // Delinquent,
    TrialExpired,
    Paused,
    Suspended, // due to non-payment
    Cancelled,
    Completed,
    Superseded, // upgrade/downgrade
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
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::ScheduledEventStatus"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum ScheduledEventStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
}
