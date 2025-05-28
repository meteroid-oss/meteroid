use error_stack::Report;

use uuid::Uuid;
// TODO duplicate as well
use super::enums::{BillingPeriodEnum, BillingType, SubscriptionFeeBillingPeriod};

use crate::domain::SubscriptionFee;
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::ids::{BaseId, BillableMetricId, PlanVersionId, PriceComponentId, ProductId};
use diesel_models::price_components::{PriceComponentRow, PriceComponentRowNew};
use golden::golden;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct PriceComponent {
    pub id: PriceComponentId,
    pub name: String,
    pub fee: FeeType,
    pub product_id: Option<ProductId>,
}

impl TryInto<PriceComponent> for PriceComponentRow {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<PriceComponent, Self::Error> {
        let fee: FeeType = self.fee.try_into()?;

        // TODO we also have plan version id and metric id in the type
        Ok(PriceComponent {
            id: self.id,
            name: self.name,
            fee,
            product_id: self.product_id,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PriceComponentNew {
    pub name: String,
    pub fee: FeeType,
    pub product_id: Option<ProductId>,
    pub plan_version_id: PlanVersionId,
}

#[derive(Debug, Clone)]
pub struct PriceComponentNewInternal {
    pub name: String,
    pub fee: FeeType,
    pub product_id: Option<ProductId>,
}

impl TryInto<PriceComponentRowNew> for PriceComponentNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<PriceComponentRowNew, Self::Error> {
        let metric = self.fee.metric_id();

        let json_fee: serde_json::Value = self.fee.try_into()?;

        Ok(PriceComponentRowNew {
            id: PriceComponentId::new(),
            plan_version_id: self.plan_version_id,
            name: self.name,
            fee: json_fee,
            product_id: self.product_id,
            billable_metric_id: metric,
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
    Matrix {
        rates: Vec<MatrixRow>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MatrixRow {
    pub dimension1: MatrixDimension,
    pub dimension2: Option<MatrixDimension>,
    pub per_unit_price: rust_decimal::Decimal,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MatrixDimension {
    pub key: String,
    pub value: String,
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
        metric_id: BillableMetricId,
        thresholds: Vec<CapacityThreshold>,
    },
    Usage {
        metric_id: BillableMetricId,
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

json_value_serde!(FeeType);

impl FeeType {
    pub fn metric_id(&self) -> Option<BillableMetricId> {
        match self {
            FeeType::Capacity { metric_id, .. } => Some(*metric_id),
            FeeType::Usage { metric_id, .. } => Some(*metric_id),
            _ => None,
        }
    }

    pub fn to_subscription_fee(
        &self,
    ) -> Result<(SubscriptionFeeBillingPeriod, SubscriptionFee), StoreError> {
        match self {
            FeeType::Rate { rates } => {
                if rates.len() != 1 {
                    return Err(StoreError::InvalidArgument(format!(
                        "Expected a single rate or a parametrized component, found: {}",
                        rates.len()
                    )));
                }
                Ok((
                    rates[0].term.as_subscription_billing_period(),
                    SubscriptionFee::Rate {
                        rate: rates[0].price,
                    },
                ))
            }
            FeeType::Slot {
                minimum_count,
                quota,
                slot_unit_name,
                rates,
                ..
            } => {
                if rates.len() != 1 {
                    return Err(StoreError::InvalidArgument(format!(
                        "Expected a single rate or a parametrized component, found: {}",
                        rates.len()
                    )));
                }

                Ok((
                    rates[0].term.as_subscription_billing_period(),
                    SubscriptionFee::Slot {
                        unit: slot_unit_name.clone(),
                        unit_rate: rates[0].price,
                        min_slots: *minimum_count,
                        max_slots: *quota,
                        initial_slots: minimum_count.unwrap_or(0),
                    },
                ))
            }
            FeeType::Capacity {
                metric_id,
                thresholds,
            } => {
                if thresholds.len() != 1 {
                    return Err(StoreError::InvalidArgument(format!(
                        "Expected either a single threshold or a parametrized component, found: {}",
                        thresholds.len()
                    )));
                }

                Ok((
                    SubscriptionFeeBillingPeriod::Monthly,
                    SubscriptionFee::Capacity {
                        metric_id: *metric_id,
                        overage_rate: thresholds[0].per_unit_overage,
                        included: thresholds[0].included_amount,
                        rate: thresholds[0].price,
                    },
                ))
            }

            FeeType::OneTime {
                quantity,
                unit_price,
            } => Ok((
                SubscriptionFeeBillingPeriod::OneTime,
                SubscriptionFee::OneTime {
                    rate: *unit_price,
                    quantity: *quantity,
                },
            )),
            FeeType::Usage { metric_id, pricing } => Ok((
                SubscriptionFeeBillingPeriod::Monthly,
                SubscriptionFee::Usage {
                    metric_id: *metric_id,
                    model: pricing.clone(),
                },
            )),
            FeeType::ExtraRecurring {
                cadence,
                unit_price,
                quantity,
                billing_type,
            } => Ok((
                cadence.as_subscription_billing_period(),
                SubscriptionFee::Recurring {
                    rate: *unit_price,
                    quantity: *quantity,
                    billing_type: billing_type.clone(),
                },
            )),
        }
    }

    pub fn to_subscription_fee_parameterized(
        &self,
        initial_slot_count: &Option<u32>,
        billing_period: &Option<BillingPeriodEnum>,
        committed_capacity: &Option<u64>,
    ) -> Result<(SubscriptionFeeBillingPeriod, SubscriptionFee), StoreError> {
        match self {
            FeeType::Rate { rates } => {
                if initial_slot_count.is_some() || committed_capacity.is_some() {
                    return Err(StoreError::InvalidArgument(
                        "Unexpected parameters for rate fee".to_string(),
                    ));
                }

                if let Some(billing_period) = &billing_period {
                    let rate = rates
                        .iter()
                        .find(|r| &r.term == billing_period)
                        .ok_or_else(|| {
                            StoreError::InvalidArgument(format!(
                                "Rate not found for billing period: {:?}",
                                billing_period
                            ))
                        })?;
                    Ok((
                        billing_period.as_subscription_billing_period(),
                        SubscriptionFee::Rate { rate: rate.price },
                    ))
                } else {
                    if rates.len() != 1 {
                        return Err(StoreError::InvalidArgument(format!(
                            "Expected a single rate, found: {}",
                            rates.len()
                        )));
                    }

                    let rate = &rates[0];
                    Ok((
                        rate.term.as_subscription_billing_period(),
                        SubscriptionFee::Rate { rate: rate.price },
                    ))
                }
            }
            FeeType::Slot {
                rates,
                minimum_count,
                slot_unit_name,
                quota,
                ..
            } => {
                let billing_period = billing_period.as_ref().ok_or_else(|| {
                    StoreError::InvalidArgument("Missing billing period".to_string())
                })?;

                let rate = rates
                    .iter()
                    .find(|r| &r.term == billing_period)
                    .ok_or_else(|| {
                        StoreError::InvalidArgument(format!(
                            "Rate not found for billing period: {:?}",
                            billing_period
                        ))
                    })?;
                let initial_slots =
                    initial_slot_count.unwrap_or_else(|| minimum_count.unwrap_or(0));

                if committed_capacity.is_some() {
                    return Err(StoreError::InvalidArgument(
                        "Unexpected committed capacity for slot fee".to_string(),
                    ));
                }
                Ok((
                    billing_period.as_subscription_billing_period(),
                    SubscriptionFee::Slot {
                        unit: slot_unit_name.clone(),
                        unit_rate: rate.price,
                        min_slots: *minimum_count,
                        max_slots: *quota,
                        initial_slots,
                    },
                ))
            }
            FeeType::Capacity {
                metric_id,
                thresholds,
            } => {
                let committed_capacity = committed_capacity.ok_or_else(|| {
                    StoreError::InvalidArgument("Missing committed capacity".to_string())
                })?;

                let threshold = thresholds
                    .iter()
                    .find(|t| t.included_amount == committed_capacity)
                    .ok_or_else(|| {
                        StoreError::InvalidArgument(format!(
                            "Threshold not found for committed capacity: {}",
                            committed_capacity
                        ))
                    })?;

                if billing_period.is_some() || initial_slot_count.is_some() {
                    return Err(StoreError::InvalidArgument(
                        "Unexpected parameters for capacity fee".to_string(),
                    ));
                }

                Ok((
                    SubscriptionFeeBillingPeriod::Monthly, // Default to monthly, until we support period parametrization for capacity
                    SubscriptionFee::Capacity {
                        metric_id: *metric_id,
                        overage_rate: threshold.per_unit_overage,
                        included: threshold.included_amount,
                        rate: threshold.price,
                    },
                ))
            }
            // all other case should fail, as they just cannot be parametrized
            FeeType::Usage { .. } | FeeType::ExtraRecurring { .. } | FeeType::OneTime { .. } => {
                Err(StoreError::InvalidArgument(format!(
                    "Cannot parameterize fee type: {:?}",
                    self
                )))
            }
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

golden!(FeeType, {
    "rate" => FeeType::Rate {
        rates: vec![TermRate {
            term: BillingPeriodEnum::Monthly,
            price: rust_decimal::Decimal::new(100, 2),
        }],
    },
    "slot" => FeeType::Slot {
        rates: vec![TermRate {
            term: BillingPeriodEnum::Monthly,
            price: rust_decimal::Decimal::new(100, 2),
        }],
        slot_unit_name: "slot".to_string(),
        upgrade_policy: UpgradePolicy::Prorated,
        downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
        minimum_count: Some(1),
        quota: Some(10),
    },
    "capacity" => FeeType::Capacity {
        metric_id: Uuid::nil().into(),
        thresholds: vec![CapacityThreshold {
            included_amount: 100,
            price: rust_decimal::Decimal::new(100, 2),
            per_unit_overage: rust_decimal::Decimal::new(10, 2),
        }],
    },
    "usage_per_unit" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::PerUnit {
            rate: rust_decimal::Decimal::new(100, 2),
        },
    },
    "usage_tiered" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::Tiered {
            tiers: vec![TierRow {
                first_unit: 1,
                rate: rust_decimal::Decimal::new(100, 2),
                flat_fee: None,
                flat_cap: None,
            }],
            block_size: Some(10),
        },
    },
    "usage_volume" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::Volume {
            tiers: vec![TierRow {
                first_unit: 1,
                rate: rust_decimal::Decimal::new(100, 2),
                flat_fee: None,
                flat_cap: None,
            }],
            block_size: Some(10),
        },
    },
    "usage_package" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::Package {
            block_size: 10,
            rate: rust_decimal::Decimal::new(100, 2),
        },
    },
    "usage_matrix" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::Matrix {
            rates: vec![MatrixRow {
                dimension1: MatrixDimension {
                    key: "key".to_string(),
                    value: "value".to_string(),
                },
                dimension2: None,
                per_unit_price: rust_decimal::Decimal::new(100, 2),
            }],
        },
    },
    "extra_recurring" => FeeType::ExtraRecurring {
        unit_price: rust_decimal::Decimal::new(100, 2),
        quantity: 1,
        billing_type: BillingType::Advance,
        cadence: BillingPeriodEnum::Monthly,
    },
    "one_time" => FeeType::OneTime {
        unit_price: rust_decimal::Decimal::new(100, 2),
        quantity: 1,
    },

});
