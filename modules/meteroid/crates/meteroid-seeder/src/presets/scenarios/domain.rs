use chrono::NaiveDate;
use educe::Educe;
use meteroid_store::StoreResult;
use meteroid_store::domain::{
    BillingMetricAggregateEnum, BillingPeriodEnum, BillingType, CapacityThreshold, DowngradePolicy,
    PlanTypeEnum, SegmentationMatrix, TermRate, UnitConversionRoundingEnum, UpgradePolicy,
    UsagePricingModel,
};
use meteroid_store::errors::StoreError;

#[derive(Clone)]
pub struct Scenario {
    pub name: String, // will be the tenant name
    pub metrics: Vec<BillableMetric>,
    pub plans: Vec<Plan>,
    pub customers: Vec<Customer>,
}

#[derive(Clone)]
pub struct Plan {
    pub name: String,
    pub currency: String,
    pub plan_type: PlanTypeEnum,
    pub components: Vec<PriceComponent>,
}

#[derive(Clone, Educe)]
#[educe(Default)]
pub struct BillableMetric {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    #[educe(Default = BillingMetricAggregateEnum::Sum)]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    pub segmentation_matrix: Option<SegmentationMatrix>,
    pub usage_group_key: Option<String>,
}

#[derive(Clone)]
pub struct Subscription {
    pub plan_name: String,
    pub start_date: NaiveDate,
}

#[derive(Clone)]
pub struct Customer {
    pub name: String,
    pub email: String,
    pub currency: String,
    pub subscription: Subscription,
}

#[derive(Clone)]
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
        metric_code: String,
        thresholds: Vec<CapacityThreshold>,
    },
    Usage {
        metric_code: String,
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
    pub fn metric_code(&self) -> Option<String> {
        match self {
            FeeType::Capacity { metric_code, .. } => Some(metric_code.clone()),
            FeeType::Usage { metric_code, .. } => Some(metric_code.clone()),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct PriceComponent {
    pub name: String,
    pub fee: FeeType,
}

impl PriceComponent {
    pub fn to_domain(
        &self,
        metrics: &[meteroid_store::domain::BillableMetric],
    ) -> StoreResult<meteroid_store::domain::PriceComponentNewInternal> {
        let maybe_metric_code = self.fee.metric_code();

        let maybe_metric = maybe_metric_code
            .and_then(|metric_code| metrics.iter().find(|metric| metric.code == metric_code));

        let domain_fee: meteroid_store::domain::FeeType = match &self.fee {
            FeeType::Rate { rates } => meteroid_store::domain::FeeType::Rate {
                rates: rates.clone(),
            },
            FeeType::Slot {
                rates,
                slot_unit_name,
                upgrade_policy,
                downgrade_policy,
                minimum_count,
                quota,
            } => meteroid_store::domain::FeeType::Slot {
                rates: rates.clone(),
                slot_unit_name: slot_unit_name.clone(),
                upgrade_policy: upgrade_policy.clone(),
                downgrade_policy: downgrade_policy.clone(),
                minimum_count: *minimum_count,
                quota: *quota,
            },
            FeeType::Capacity {
                metric_code,
                thresholds,
            } => {
                let metric = maybe_metric.ok_or(StoreError::ValueNotFound(format!(
                    "Metric was not found {}",
                    metric_code
                )))?;
                meteroid_store::domain::FeeType::Capacity {
                    metric_id: metric.id,
                    thresholds: thresholds.clone(),
                }
            }
            FeeType::Usage {
                metric_code,
                pricing,
            } => {
                let metric = maybe_metric.ok_or(StoreError::ValueNotFound(format!(
                    "Metric was not found {}",
                    metric_code
                )))?;
                meteroid_store::domain::FeeType::Usage {
                    metric_id: metric.id,
                    pricing: pricing.clone(),
                }
            }
            FeeType::ExtraRecurring {
                unit_price,
                quantity,
                billing_type,
                cadence,
            } => meteroid_store::domain::FeeType::ExtraRecurring {
                unit_price: *unit_price,
                quantity: *quantity,
                billing_type: billing_type.clone(),
                cadence: cadence.clone(),
            },
            FeeType::OneTime {
                unit_price,
                quantity,
            } => meteroid_store::domain::FeeType::OneTime {
                unit_price: *unit_price,
                quantity: *quantity,
            },
        };

        Ok(meteroid_store::domain::PriceComponentNewInternal {
            name: self.name.clone(),
            fee: domain_fee,
            product_id: None,
        })
    }
}
