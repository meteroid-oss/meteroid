use super::enums::{BillingPeriodEnum, BillingType, SubscriptionFeeBillingPeriod};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::UsagePricingModel;
use crate::errors::StoreError;

pub trait SubscriptionFeeInterface {
    fn price_component_id(&self) -> Option<Uuid>;
    fn product_id(&self) -> Option<Uuid>;
    fn subscription_id(&self) -> Uuid;
    fn name_ref(&self) -> &String;
    fn period_ref(&self) -> &SubscriptionFeeBillingPeriod;
    fn fee_ref(&self) -> &SubscriptionFee;
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SubscriptionComponent {
    pub id: Uuid,
    pub price_component_id: Option<Uuid>,
    pub product_id: Option<Uuid>,
    pub subscription_id: Uuid,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
}

impl SubscriptionFeeInterface for SubscriptionComponent {
    #[inline]
    fn price_component_id(&self) -> Option<Uuid> {
        self.price_component_id
    }

    #[inline]
    fn product_id(&self) -> Option<Uuid> {
        self.product_id
    }

    #[inline]
    fn subscription_id(&self) -> Uuid {
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
}

impl TryInto<SubscriptionComponent> for SubscriptionComponentRow {
    type Error = StoreError;

    fn try_into(self) -> Result<SubscriptionComponent, Self::Error> {
        let decoded_fee: SubscriptionFee = serde_json::from_value(self.fee)
            .map_err(|e| StoreError::SerdeError("Failed to deserialize fee".to_string(), e))?;

        Ok(SubscriptionComponent {
            id: self.id,
            price_component_id: self.price_component_id,
            product_id: self.product_id,
            subscription_id: self.subscription_id,
            name: self.name,
            period: self.period.into(),
            fee: decoded_fee,
        })
    }
}

impl SubscriptionComponent {
    pub fn metric_id(&self) -> Option<Uuid> {
        self.fee.metric_id()
    }

    pub fn is_standard(&self) -> bool {
        self.fee.is_standard()
    }
}

#[derive(Debug)]
pub struct SubscriptionComponentNew {
    pub subscription_id: Uuid,
    pub internal: SubscriptionComponentNewInternal,
}

impl TryInto<SubscriptionComponentRowNew> for SubscriptionComponentNew {
    type Error = StoreError;

    fn try_into(self) -> Result<SubscriptionComponentRowNew, Self::Error> {
        let fee = serde_json::to_value(self.internal.fee)
            .map_err(|e| StoreError::SerdeError("Failed to serialize fee".to_string(), e))?;

        Ok(SubscriptionComponentRowNew {
            id: Uuid::now_v7(),
            subscription_id: self.subscription_id,
            price_component_id: self.internal.price_component_id,
            product_id: self.internal.product_id,
            name: self.internal.name,
            period: self.internal.period.into(),
            fee,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CreateSubscriptionComponents {
    pub parameterized_components: Vec<ComponentParameterization>,
    pub overridden_components: Vec<ComponentOverride>,
    pub extra_components: Vec<ExtraComponent>,
    pub remove_components: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct ComponentParameterization {
    pub component_id: Uuid,
    pub parameters: ComponentParameters,
}

#[derive(Debug, Clone)]
pub struct ComponentParameters {
    pub initial_slot_count: Option<u32>,
    pub billing_period: Option<BillingPeriodEnum>,
    pub committed_capacity: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ComponentOverride {
    pub component_id: Uuid,
    pub component: SubscriptionComponentNewInternal,
}

#[derive(Debug, Clone)]
pub struct ExtraComponent {
    pub component: SubscriptionComponentNewInternal,
}

#[derive(Debug, Clone)]
pub struct SubscriptionComponentNewInternal {
    pub price_component_id: Option<Uuid>,
    pub product_id: Option<Uuid>,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    // pub mrr_value: Option<rust_decimal::Decimal>, // TODO
    pub fee: SubscriptionFee,
    pub is_override: bool,
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
        metric_id: Uuid,
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
        metric_id: Uuid,
        model: UsagePricingModel,
    },
}

impl SubscriptionFee {
    pub fn metric_id(&self) -> Option<Uuid> {
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
