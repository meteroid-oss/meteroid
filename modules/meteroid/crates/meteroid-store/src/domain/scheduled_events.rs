use crate::domain::BillingPeriodEnum;
use crate::domain::enums::{ScheduledEventTypeEnum, SubscriptionFeeBillingPeriod};
use crate::domain::subscription_components::SubscriptionFee;
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use chrono::NaiveDateTime;
use common_domain::actor::{Actor, ActorType};
use common_domain::ids::{
    AddOnId, BaseId, PlanVersionId, PriceComponentId, PriceId, ProductId, ScheduledEventId,
    SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use diesel_models::enums::SubscriptionChangeKindEnum;
use diesel_models::subscription_changes::{SubscriptionChangeRow, SubscriptionChangeRowNew};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A plan change or amendment scheduled for a later date, as it was requested.
#[derive(Clone, Debug)]
pub struct ScheduledEvent {
    pub id: ScheduledEventId,
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub event_type: ScheduledEventTypeEnum,
    pub scheduled_time: NaiveDateTime,
    pub event_data: ScheduledEventData,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub withdrawn_at: Option<NaiveDateTime>,
    pub source: String,
    pub created_by_customer: bool,
    pub actor_type: ActorType,
    pub actor_uuid: Option<Uuid>,
    pub actor_alias: Option<String>,
}

impl ScheduledEvent {
    /// The actor that scheduled the change, rebuilt from its persisted columns.
    pub fn created_by(&self) -> Actor {
        Actor::from_parts(self.actor_type, self.actor_uuid, self.actor_alias.clone())
    }
}

impl TryFrom<SubscriptionChangeRow> for ScheduledEvent {
    type Error = StoreErrorReport;

    fn try_from(row: SubscriptionChangeRow) -> Result<Self, Self::Error> {
        Ok(ScheduledEvent {
            id: row.id,
            subscription_id: row.subscription_id,
            tenant_id: row.tenant_id,
            event_type: row.kind.into(),
            scheduled_time: row.effective_at,
            event_data: row.change.try_into()?,
            created_at: row.created_at,
            updated_at: row.updated_at,
            withdrawn_at: row.withdrawn_at,
            source: row.source,
            created_by_customer: row.created_by_customer,
            actor_type: row.actor_type.into(),
            actor_uuid: row.actor_uuid,
            actor_alias: row.actor_alias,
        })
    }
}

/// A change to schedule: written as facts at once, recorded with the rows it wrote.
#[derive(Clone, Debug)]
pub struct ScheduledEventNew {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub scheduled_time: NaiveDateTime,
    pub event_data: ScheduledEventData,
    pub source: String,
    /// The actor scheduling the change, decomposed into the persisted actor columns.
    pub created_by: Actor,
}

impl ScheduledEventNew {
    /// The change as recorded once written, before its record is stored.
    pub fn into_event(self, now: NaiveDateTime) -> ScheduledEvent {
        ScheduledEvent {
            id: ScheduledEventId::new(),
            subscription_id: self.subscription_id,
            tenant_id: self.tenant_id,
            event_type: self.event_data.to_event_type_enum(),
            scheduled_time: self.scheduled_time,
            created_at: now,
            updated_at: now,
            withdrawn_at: None,
            source: self.source,
            created_by_customer: self.created_by.is_customer(),
            actor_type: self.created_by.actor_type(),
            actor_uuid: self.created_by.as_uuid(),
            actor_alias: self.created_by.actor_alias(),
            event_data: self.event_data,
        }
    }
}

impl ScheduledEvent {
    /// Its record, with the rows it wrote as facts.
    pub fn to_row(
        &self,
        written_facts: serde_json::Value,
    ) -> Result<SubscriptionChangeRowNew, StoreErrorReport> {
        let change = serde_json::to_value(&self.event_data).map_err(|error| {
            error_stack::Report::new(StoreError::SerdeError(
                "Failed to serialize ScheduledEventData".to_string(),
                error,
            ))
        })?;
        Ok(SubscriptionChangeRowNew {
            id: self.id,
            tenant_id: self.tenant_id,
            subscription_id: self.subscription_id,
            kind: self.event_data.kind(),
            effective_at: self.scheduled_time,
            change,
            written_facts: Some(written_facts),
            source: self.source.clone(),
            created_by_customer: self.created_by_customer,
            actor_type: self.actor_type.into(),
            actor_uuid: self.actor_uuid,
            actor_alias: self.actor_alias.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScheduledEventData {
    ApplyPlanChange {
        #[serde(default)]
        source_plan_version_id: Option<PlanVersionId>,
        new_plan_version_id: PlanVersionId,
        /// The option of the new version the subscription bills from the change.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_plan_option_id: Option<common_domain::ids::PlanOptionId>,
        component_mappings: Vec<ComponentMapping>,
    },
    /// A manual/sales-led amendment at the end of the current period, its component and add-on
    /// deltas fully resolved (no plan-version switch).
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
            Self::ApplyPlanChange { .. } => ScheduledEventTypeEnum::ApplyPlanChange,
            Self::ApplyAmendment { .. } => ScheduledEventTypeEnum::ApplyAmendment,
        }
    }

    pub fn kind(&self) -> SubscriptionChangeKindEnum {
        match self {
            Self::ApplyPlanChange { .. } => SubscriptionChangeKindEnum::PlanChange,
            Self::ApplyAmendment { .. } => SubscriptionChangeKindEnum::Amendment,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComponentMapping {
    /// Component matched between current and target plans: the same product billed the same way
    Matched {
        current_component_id: SubscriptionPriceComponentId,
        target_component_id: PriceComponentId,
        product_id: ProductId,
        price_id: PriceId,
        name: String,
        fee: SubscriptionFee,
        period: SubscriptionFeeBillingPeriod,
        /// The seats a change of a seat fee sets, when it sets them; else they carry over (R-108).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seats: Option<u32>,
    },
    /// Component in target plan but not in current (new)
    Added {
        target_component_id: PriceComponentId,
        product_id: Option<ProductId>,
        price_id: Option<PriceId>,
        name: String,
        fee: SubscriptionFee,
        period: SubscriptionFeeBillingPeriod,
        /// The seats the change sets on a seat fee, when it sets them: a quantity the
        /// subscription holds carries its count otherwise (R-108, R-137).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seats: Option<u32>,
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
