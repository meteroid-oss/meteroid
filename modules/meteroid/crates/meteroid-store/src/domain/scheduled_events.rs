use crate::domain::BillingPeriodEnum;
use crate::domain::enums::{
    ScheduledEventStatus, ScheduledEventTypeEnum, SubscriptionFeeBillingPeriod,
};
use crate::domain::subscription_components::SubscriptionFee;
use crate::errors::StoreErrorReport;
use crate::json_value_serde;
use chrono::NaiveDateTime;
use common_domain::ids::{
    AddOnId, BaseId, InvoiceId, PlanVersionId, PriceComponentId, PriceId, ProductId,
    ScheduledEventId, SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::scheduled_events::ScheduledEventRowNew;
use o2o::o2o;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, o2o)]
#[try_from_owned(ScheduledEventRow, StoreErrorReport)]
pub struct ScheduledEvent {
    pub id: ScheduledEventId,
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    #[from(~.into())]
    pub event_type: ScheduledEventTypeEnum,
    pub scheduled_time: NaiveDateTime,
    pub priority: i32,
    #[from(~.try_into()?)]
    pub event_data: ScheduledEventData,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    #[from(~.into())]
    pub status: ScheduledEventStatus,
    pub retries: i32,
    pub last_retry_at: Option<NaiveDateTime>,
    pub error: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub source: String, // API, System, etc.
    pub created_by_customer: bool,
}

#[derive(Clone, Debug, o2o)]
#[owned_try_into(ScheduledEventRowNew, StoreErrorReport)]
#[ghosts(
    id: common_domain::ids::ScheduledEventId::new(),
    event_type: @.event_data.to_event_type_enum().into(),
    status: diesel_models::enums::ScheduledEventStatus::Pending,
    priority: 0,
    retries: 0
)] // TODO drop priority if unused
pub struct ScheduledEventNew {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub scheduled_time: NaiveDateTime,
    #[into(~.clone().try_into()?)]
    pub event_data: ScheduledEventData,
    pub source: String,
    pub created_by_customer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScheduledEventData {
    CancelSubscription {
        reason: Option<String>,
    },
    PauseSubscription,
    FinalizeInvoice {
        invoice_id: InvoiceId,
    },
    RetryPayment {
        invoice_id: InvoiceId,
    },
    ApplyPlanChange {
        #[serde(default)]
        source_plan_version_id: Option<PlanVersionId>,
        new_plan_version_id: PlanVersionId,
        component_mappings: Vec<ComponentMapping>,
    },
    /// End paid trial - transitions subscription from TrialActive to Active
    /// Billing continues normally via RenewSubscription, this just handles the status change
    EndTrial,
    /// Apply a manual/sales-led amendment at the end of the current period.
    /// Carries fully-resolved component and add-on deltas (no plan-version switch).
    ApplyAmendment {
        component_close: Vec<SubscriptionPriceComponentId>,
        component_insert: Vec<ResolvedComponentInsert>,
        addon_close: Vec<SubscriptionAddOnId>,
        addon_insert: Vec<ResolvedAddOnInsert>,
    },
}

/// A fully-resolved subscription component to insert when an amendment is applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedComponentInsert {
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub is_override: bool,
    pub price_id: Option<PriceId>,
    /// Lineage root of the component being overridden, carried onto the new row so
    /// amendment credits stay matched to the originally-billed invoice line. `None`
    /// for genuinely new (extra) components, which become their own root.
    #[serde(default)]
    pub lineage_id: Option<SubscriptionPriceComponentId>,
    /// Pre-generated id to insert the row with, so the adjustment invoice issued
    /// in the same immediate amendment can stamp this id onto its charge line and
    /// a later removal can credit it. `None` lets the row generate its own id.
    #[serde(default)]
    pub subscription_component_id: Option<SubscriptionPriceComponentId>,
}

/// A fully-resolved subscription add-on to insert when an amendment is applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedAddOnInsert {
    pub add_on_id: AddOnId,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
    pub quantity: i32,
    /// Lineage root of the add-on being overridden, carried onto the new row. `None`
    /// for genuinely new add-ons, which become their own root.
    #[serde(default)]
    pub lineage_id: Option<SubscriptionAddOnId>,
    /// Pre-generated id to insert the row with. The add-on analogue of
    /// `ResolvedComponentInsert::subscription_component_id`.
    #[serde(default)]
    pub subscription_add_on_id: Option<SubscriptionAddOnId>,
}

json_value_serde!(ScheduledEventData);

impl ScheduledEventData {
    pub fn to_event_type_enum(&self) -> ScheduledEventTypeEnum {
        match self {
            Self::CancelSubscription { .. } => ScheduledEventTypeEnum::CancelSubscription,
            Self::PauseSubscription { .. } => ScheduledEventTypeEnum::PauseSubscription,
            Self::FinalizeInvoice { .. } => ScheduledEventTypeEnum::FinalizeInvoice,
            Self::RetryPayment { .. } => ScheduledEventTypeEnum::RetryPayment,
            Self::ApplyPlanChange { .. } => ScheduledEventTypeEnum::ApplyPlanChange,
            Self::EndTrial => ScheduledEventTypeEnum::EndTrial,
            Self::ApplyAmendment { .. } => ScheduledEventTypeEnum::ApplyAmendment,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComponentMapping {
    /// Component matched by product_id between current and target plans
    Matched {
        current_component_id: SubscriptionPriceComponentId,
        target_component_id: PriceComponentId,
        product_id: ProductId,
        price_id: PriceId,
        name: String,
        fee: SubscriptionFee,
        period: SubscriptionFeeBillingPeriod,
    },
    /// Component in target plan but not in current (new)
    Added {
        target_component_id: PriceComponentId,
        product_id: Option<ProductId>,
        price_id: Option<PriceId>,
        name: String,
        fee: SubscriptionFee,
        period: SubscriptionFeeBillingPeriod,
    },
    /// Component in current plan but not in target (to be removed)
    Removed {
        current_component_id: SubscriptionPriceComponentId,
    },
}

impl ComponentMapping {
    /// Derive the minimum billing period from a set of component mappings.
    /// Returns `None` if there are no recurring components (all OneTime or all Removed).
    pub fn derive_billing_period(mappings: &[ComponentMapping]) -> Option<BillingPeriodEnum> {
        mappings
            .iter()
            .filter_map(|m| match m {
                ComponentMapping::Matched { period, .. }
                | ComponentMapping::Added { period, .. } => period.as_billing_period_opt(),
                ComponentMapping::Removed { .. } => None,
            })
            .min()
    }
}

impl ScheduledEventNew {
    /// Creates an EndTrial scheduled event for paid trials.
    ///
    /// This event transitions a subscription from TrialActive to Active when the trial period ends.
    /// Billing continues normally via RenewSubscription - this only handles the status change.
    ///
    /// Returns `None` if the trial end date cannot be computed (invalid date arithmetic).
    pub fn end_trial(
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        billing_start_date: chrono::NaiveDate,
        trial_days: i32,
        source: impl Into<String>,
    ) -> Option<Self> {
        let trial_end_date = billing_start_date + chrono::Duration::days(i64::from(trial_days));
        let scheduled_time = trial_end_date.and_hms_opt(0, 0, 0)?;

        Some(Self {
            subscription_id,
            tenant_id,
            scheduled_time,
            event_data: ScheduledEventData::EndTrial,
            source: source.into(),
            created_by_customer: false,
        })
    }
}
