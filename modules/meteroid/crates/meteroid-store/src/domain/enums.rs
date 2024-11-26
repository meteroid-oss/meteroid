use crate::errors::StoreError;
use crate::StoreResult;
use diesel_models::enums as diesel_enums;
use error_stack::ResultExt;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::str::FromStr;
use strum::{Display, EnumIter, EnumString};

#[derive(o2o, Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
#[map_owned(diesel_enums::ActionAfterTrialEnum)]
pub enum ActionAfterTrialEnum {
    Block,
    Charge,
    Downgrade,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone)]
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

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::BillingPeriodEnum)]
pub enum BillingPeriodEnum {
    Monthly,
    Quarterly,
    Annual,
}

impl BillingPeriodEnum {
    pub fn as_months(&self) -> u32 {
        match self {
            BillingPeriodEnum::Monthly => 1,
            BillingPeriodEnum::Quarterly => 3,
            BillingPeriodEnum::Annual => 12,
        }
    }

    pub fn as_subscription_billing_period(&self) -> SubscriptionFeeBillingPeriod {
        match self {
            BillingPeriodEnum::Monthly => SubscriptionFeeBillingPeriod::Monthly,
            BillingPeriodEnum::Quarterly => SubscriptionFeeBillingPeriod::Quarterly,
            BillingPeriodEnum::Annual => SubscriptionFeeBillingPeriod::Annual,
        }
    }
}

impl PartialOrd for BillingPeriodEnum {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.as_months().cmp(&other.as_months()))
    }
}

impl Ord for BillingPeriodEnum {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_months().cmp(&other.as_months())
    }
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone)]
#[map_owned(diesel_enums::CreditNoteStatus)]
pub enum CreditNoteStatus {
    Draft,
    Finalized,
    Voided,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone)]
#[map_owned(diesel_enums::FangTaskState)]
pub enum FangTaskState {
    New,
    InProgress,
    Failed,
    Finished,
    Retried,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[map_owned(diesel_enums::InvoiceExternalStatusEnum)]
pub enum InvoiceExternalStatusEnum {
    Deleted,
    Draft,
    Finalized,
    Paid,
    PaymentFailed,
    Uncollectible,
    Void,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Copy, Eq, PartialEq)]
#[map_owned(diesel_enums::InvoiceStatusEnum)]
pub enum InvoiceStatusEnum {
    Draft,
    Finalized,
    Pending,
    Void,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[map_owned(diesel_enums::InvoiceType)]
pub enum InvoiceType {
    Recurring,
    OneOff,
    Adjustment,
    // Imported,
    UsageThreshold,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[map_owned(diesel_enums::InvoicingProviderEnum)]
pub enum InvoicingProviderEnum {
    Stripe,
    Manual,
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

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[map_owned(diesel_enums::OrganizationUserRole)]
pub enum OrganizationUserRole {
    Admin,
    Member,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[map_owned(diesel_enums::OutboxStatus)]
pub enum OutboxStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

#[derive(o2o, Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
#[map_owned(diesel_enums::PlanStatusEnum)]
pub enum PlanStatusEnum {
    #[default]
    Draft,
    Active,
    Inactive,
    Archived,
}

#[derive(o2o, Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
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

#[derive(Debug, Clone, PartialEq, Display, EnumIter, EnumString)]
pub enum WebhookOutEventTypeEnum {
    #[strum(serialize = "customer.created")]
    CustomerCreated,
    #[strum(serialize = "subscription.created")]
    SubscriptionCreated,
    #[strum(serialize = "invoice.created")]
    InvoiceCreated,
    #[strum(serialize = "invoice.finalized")]
    InvoiceFinalized,
}

impl WebhookOutEventTypeEnum {
    pub fn from_svix_channels(
        channels: &Option<Vec<String>>,
    ) -> StoreResult<Vec<WebhookOutEventTypeEnum>> {
        channels
            .as_ref()
            .unwrap_or(&vec![])
            .iter()
            .map(|x| WebhookOutEventTypeEnum::from_str(x.as_str()))
            .collect::<Result<Vec<_>, _>>()
            .change_context(StoreError::WebhookServiceError(
                "invalid webhook event type".into(),
            ))
    }

    pub fn group(&self) -> String {
        match self {
            WebhookOutEventTypeEnum::CustomerCreated => "customer".to_string(),
            WebhookOutEventTypeEnum::SubscriptionCreated => "subscription".to_string(),
            WebhookOutEventTypeEnum::InvoiceCreated => "invoice".to_string(),
            WebhookOutEventTypeEnum::InvoiceFinalized => "invoice".to_string(),
        }
    }
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone)]
#[map_owned(diesel_enums::SubscriptionEventType)]
pub enum SubscriptionEventType {
    Created,
    Activated,
    Switch,
    Cancelled,
    Reactivated,
    Updated,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[map_owned(diesel_enums::SubscriptionFeeBillingPeriod)]
pub enum SubscriptionFeeBillingPeriod {
    OneTime,
    Monthly,
    Quarterly,
    Annual,
}

impl SubscriptionFeeBillingPeriod {
    pub fn as_months(&self) -> i32 {
        match self {
            SubscriptionFeeBillingPeriod::OneTime => i32::MAX, // month_elapsed % OneTime.as_months() will only be 0 if month_elapsed is 0
            SubscriptionFeeBillingPeriod::Monthly => 1,
            SubscriptionFeeBillingPeriod::Quarterly => 3,
            SubscriptionFeeBillingPeriod::Annual => 12,
        }
    }

    pub fn as_billing_period_opt(&self) -> Option<BillingPeriodEnum> {
        match self {
            SubscriptionFeeBillingPeriod::OneTime => None,
            SubscriptionFeeBillingPeriod::Monthly => Some(BillingPeriodEnum::Monthly),
            SubscriptionFeeBillingPeriod::Quarterly => Some(BillingPeriodEnum::Quarterly),
            SubscriptionFeeBillingPeriod::Annual => Some(BillingPeriodEnum::Annual),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum BillingType {
    Advance,
    Arrears,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone)]
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
}
