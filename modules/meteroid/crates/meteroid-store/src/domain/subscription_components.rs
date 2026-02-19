use super::enums::{BillingPeriodEnum, BillingType, SubscriptionFeeBillingPeriod};
use crate::domain::UsagePricingModel;
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::ids::{
    BaseId, BillableMetricId, PriceComponentId, PriceId, ProductId, SubscriptionAddOnId,
    SubscriptionId, SubscriptionPriceComponentId,
};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use serde::{Deserialize, Serialize};

pub trait SubscriptionFeeInterface {
    fn price_component_id(&self) -> Option<PriceComponentId>;
    fn product_id(&self) -> Option<ProductId>;
    fn subscription_id(&self) -> SubscriptionId;
    fn name_ref(&self) -> &String;
    fn period_ref(&self) -> &SubscriptionFeeBillingPeriod;
    fn fee_ref(&self) -> &SubscriptionFee;
    fn sub_component_id(&self) -> Option<SubscriptionPriceComponentId>;
    fn sub_add_on_id(&self) -> Option<SubscriptionAddOnId>;
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SubscriptionComponent {
    pub id: SubscriptionPriceComponentId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub subscription_id: SubscriptionId,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub price_id: Option<PriceId>,
}

impl SubscriptionFeeInterface for SubscriptionComponent {
    #[inline]
    fn price_component_id(&self) -> Option<PriceComponentId> {
        self.price_component_id
    }

    #[inline]
    fn product_id(&self) -> Option<ProductId> {
        self.product_id
    }

    #[inline]
    fn subscription_id(&self) -> SubscriptionId {
        self.subscription_id
    }

    #[inline]
    fn name_ref(&self) -> &String {
        &self.name
    }

    #[inline]
    fn period_ref(&self) -> &SubscriptionFeeBillingPeriod {
        &self.period
    }

    #[inline]
    fn fee_ref(&self) -> &SubscriptionFee {
        &self.fee
    }

    #[inline]
    fn sub_component_id(&self) -> Option<SubscriptionPriceComponentId> {
        Some(self.id)
    }

    #[inline]
    fn sub_add_on_id(&self) -> Option<SubscriptionAddOnId> {
        None
    }
}

impl TryInto<SubscriptionComponent> for SubscriptionComponentRow {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<SubscriptionComponent, Self::Error> {
        let decoded_fee: SubscriptionFee = self
            .legacy_fee
            .ok_or_else(|| {
                StoreError::InvalidArgument(
                    "subscription_component has no legacy_fee (v2 rows are resolved by repository)"
                        .to_string(),
                )
            })?
            .try_into()?;

        Ok(SubscriptionComponent {
            id: self.id,
            price_component_id: self.price_component_id,
            product_id: self.product_id,
            subscription_id: self.subscription_id,
            name: self.name,
            period: self.period.into(),
            fee: decoded_fee,
            price_id: self.price_id,
        })
    }
}

impl SubscriptionComponent {
    pub fn metric_id(&self) -> Option<BillableMetricId> {
        self.fee.metric_id()
    }

    pub fn is_standard(&self) -> bool {
        self.fee.is_standard()
    }
}

#[derive(Debug)]
pub struct SubscriptionComponentNew {
    pub subscription_id: SubscriptionId,
    pub internal: SubscriptionComponentNewInternal,
}

impl TryInto<SubscriptionComponentRowNew> for SubscriptionComponentNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<SubscriptionComponentRowNew, Self::Error> {
        let legacy_fee: serde_json::Value = self.internal.fee.try_into()?;

        Ok(SubscriptionComponentRowNew {
            id: SubscriptionPriceComponentId::new(),
            subscription_id: self.subscription_id,
            price_component_id: self.internal.price_component_id,
            product_id: self.internal.product_id,
            name: self.internal.name,
            period: self.internal.period.into(),
            legacy_fee: Some(legacy_fee),
            price_id: self.internal.price_id,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionComponents {
    pub parameterized_components: Vec<ComponentParameterization>,
    pub overridden_components: Vec<ComponentOverride>,
    pub extra_components: Vec<ExtraComponent>,
    pub remove_components: Vec<PriceComponentId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentParameterization {
    pub component_id: PriceComponentId,
    pub parameters: ComponentParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentParameters {
    pub initial_slot_count: Option<u32>,
    pub billing_period: Option<BillingPeriodEnum>,
    pub committed_capacity: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentOverride {
    pub component_id: PriceComponentId,
    pub name: String,
    pub price_entry: crate::domain::price_components::PriceEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraComponent {
    pub name: String,
    pub product_ref: crate::domain::price_components::ProductRef,
    pub price_entry: crate::domain::price_components::PriceEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionComponentNewInternal {
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    // pub mrr_value: Option<rust_decimal::Decimal>, // TODO
    pub fee: SubscriptionFee,
    pub is_override: bool,
    pub price_id: Option<PriceId>,
}

// TODO golden tests
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SubscriptionFee {
    Rate {
        rate: rust_decimal::Decimal,
    },
    OneTime {
        rate: rust_decimal::Decimal,
        quantity: u32,
    },
    Recurring {
        rate: rust_decimal::Decimal,
        quantity: u32,
        billing_type: BillingType,
    },
    Capacity {
        rate: rust_decimal::Decimal,
        included: u64,
        overage_rate: rust_decimal::Decimal,
        metric_id: BillableMetricId,
    },
    Slot {
        unit: String,
        unit_rate: rust_decimal::Decimal,
        min_slots: Option<u32>,
        max_slots: Option<u32>,
        initial_slots: u32,
        // upgrade downgrade policies TODO
    },
    Usage {
        metric_id: BillableMetricId,
        model: UsagePricingModel,
    },
}

json_value_serde!(SubscriptionFee);

impl SubscriptionFee {
    /// Apply subscription-level parameters to an already-resolved fee.
    /// Used when an override provides the pricing and a parameterization provides
    /// runtime values like initial_slot_count.
    pub fn apply_parameters(&mut self, params: &ComponentParameters) {
        if let SubscriptionFee::Slot { initial_slots, .. } = self
            && let Some(count) = params.initial_slot_count
        {
            *initial_slots = count;
        }
    }

    pub fn metric_id(&self) -> Option<BillableMetricId> {
        match self {
            SubscriptionFee::Usage { metric_id, .. } => Some(*metric_id),
            SubscriptionFee::Capacity { metric_id, .. } => Some(*metric_id),
            _ => None,
        }
    }

    /**
     * Returns true if the component is Rate/Slot/Capacity, false otherwise.
     */
    pub fn is_standard(&self) -> bool {
        match self {
            SubscriptionFee::Rate { .. }
            | SubscriptionFee::Slot { .. }
            | SubscriptionFee::Capacity { .. } => true,
            SubscriptionFee::OneTime { .. }
            | SubscriptionFee::Recurring { .. }
            | SubscriptionFee::Usage { .. } => false,
        }
    }
}
