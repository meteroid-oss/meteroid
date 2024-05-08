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

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceExternalStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoiceExternalStatusEnum {
    Deleted,
    Draft,
    Finalized,
    Paid,
    PaymentFailed,
    Uncollectible,
    Void,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceStatusEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoiceStatusEnum {
    Draft,
    Finalized,
    Pending,
    Void,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceType"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoiceType {
    Recurring,
    OneOff,
    Adjustment,
    Imported,
    UsageThreshold,
}

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::InvoicingProviderEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum InvoicingProviderEnum {
    Stripe,
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

#[derive(diesel_derive_enum::DbEnum, Debug, Clone)]
#[ExistingTypePath = "crate::schema::sql_types::WebhookOutEventTypeEnum"]
#[DbValueStyle = "SCREAMING_SNAKE_CASE"]
pub enum WebhookOutEventTypeEnum {
    CustomerCreated,
    SubscriptionCreated,
    InvoiceCreated,
    InvoiceFinalized,
}
