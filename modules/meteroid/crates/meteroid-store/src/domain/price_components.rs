use error_stack::Report;

use uuid::Uuid;
// TODO duplicate as well
use super::enums::{BillingPeriodEnum, BillingType};

use crate::errors::StoreError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct PriceComponent {
    pub id: Uuid,
    pub name: String,
    pub fee: FeeType,
    pub product_item_id: Option<Uuid>,
}

impl TryInto<PriceComponent> for diesel_models::price_components::PriceComponent {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<PriceComponent, Self::Error> {
        let fee: FeeType = serde_json::from_value(self.fee).map_err(|e| {
            StoreError::SerdeError("Failed to deserialize price component fee".to_string(), e)
        })?;

        // TODO we also have plan version id and metric id in the type
        Ok(PriceComponent {
            id: self.id,
            name: self.name,
            fee,
            product_item_id: self.product_item_id,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PriceComponentNew {
    pub name: String,
    pub fee: FeeType,
    pub product_item_id: Option<Uuid>,
    pub plan_version_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct PriceComponentNewInternal {
    pub name: String,
    pub fee: FeeType,
    pub product_item_id: Option<Uuid>,
}

impl TryInto<diesel_models::price_components::PriceComponentNew> for PriceComponentNew {
    type Error = StoreError;

    fn try_into(self) -> Result<diesel_models::price_components::PriceComponentNew, StoreError> {
        let json_fee = serde_json::to_value(&self.fee)
            .map_err(|e| {
                StoreError::SerdeError("Failed to serialize price component fee".to_string(), e)
            })
            .unwrap();

        Ok(diesel_models::price_components::PriceComponentNew {
            id: Uuid::now_v7(),
            plan_version_id: self.plan_version_id,
            name: self.name,
            fee: json_fee,
            product_item_id: self.product_item_id,
            billable_metric_id: self.fee.metric_id(),
        })
    }
}

//
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum UsagePricingModel {
    PerUnit {
        rate: rust_decimal::Decimal,
    },
    Tiered {
        tiers: Vec<TierRow>,
        block_size: Option<u64>,
    },
    Volume {
        tiers: Vec<TierRow>,
        block_size: Option<u64>,
    },
    Package {
        block_size: u64,
        rate: rust_decimal::Decimal,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TierRow {
    pub first_unit: u64,
    // last unit is implicit.
    pub rate: rust_decimal::Decimal,
    pub flat_fee: Option<rust_decimal::Decimal>,
    pub flat_cap: Option<rust_decimal::Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FeeType {
    Rate {
        rates: Vec<TermRate>,
    },
    Slot {
        rates: Vec<TermRate>,
        slot_unit_name: String,
        upgrade_policy: UpgradePolicy,
        downgrade_policy: DowngradePolicy,
        minimum_count: Option<u32>,
        quota: Option<u32>,
    },
    Capacity {
        metric_id: Uuid,
        thresholds: Vec<CapacityThreshold>,
    },
    Usage {
        metric_id: Uuid,
        pricing: UsagePricingModel,
    },
    ExtraRecurring {
        unit_price: rust_decimal::Decimal,
        quantity: u32,
        billing_type: BillingType,
        cadence: BillingPeriodEnum,
    },
    OneTime {
        unit_price: rust_decimal::Decimal,
        quantity: u32,
    },
}

impl FeeType {
    pub fn metric_id(&self) -> Option<Uuid> {
        match self {
            FeeType::Capacity { metric_id, .. } => Some(*metric_id),
            FeeType::Usage { metric_id, .. } => Some(*metric_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermRate {
    pub term: BillingPeriodEnum,
    pub price: rust_decimal::Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityThreshold {
    pub included_amount: u64,
    pub price: rust_decimal::Decimal,
    pub per_unit_overage: rust_decimal::Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpgradePolicy {
    Prorated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DowngradePolicy {
    RemoveAtEndOfPeriod,
}
