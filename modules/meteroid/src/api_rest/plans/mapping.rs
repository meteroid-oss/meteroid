use crate::api_rest::plans::model::{
    AvailableParameters, BillingPeriodEnum, CapacityThreshold, Fee, MatrixDimension, MatrixRow,
    Plan, PriceComponent, ProductFamily, TermRate, TierRow, TrialConfig, UsagePricingModel,
};
use meteroid_store::domain;
use std::collections::HashMap;

pub fn plan_to_rest(
    plan: domain::Plan,
    version: domain::PlanVersion,
    price_components: Vec<domain::price_components::PriceComponent>,
    product_family_name: String,
) -> Plan {
    let rest_components: Vec<PriceComponent> = price_components
        .iter()
        .map(price_component_to_rest)
        .collect();

    let available_parameters = extract_available_parameters(&price_components);

    let trial = if let Some(trial_duration) = version.trial_duration_days {
        Some(TrialConfig {
            duration_days: trial_duration as u32,
            is_free: version.trial_is_free,
            trialing_plan_id: version.trialing_plan_id,
        })
    } else {
        None
    };

    Plan {
        id: plan.id,
        name: plan.name,
        description: plan.description,
        created_at: plan.created_at,
        plan_type: plan.plan_type.into(),
        status: plan.status.into(),
        product_family: ProductFamily {
            id: plan.product_family_id,
            name: product_family_name,
        },
        version_id: version.id,
        version: version.version,
        currency: version.currency,
        net_terms: version.net_terms,
        trial,
        price_components: rest_components,
        available_parameters,
    }
}

fn price_component_to_rest(component: &domain::price_components::PriceComponent) -> PriceComponent {
    PriceComponent {
        id: component.id,
        name: component.name.clone(),
        fee: fee_type_to_rest(&component.fee),
        product_id: component.product_id,
    }
}

fn fee_type_to_rest(fee: &domain::price_components::FeeType) -> Fee {
    match fee {
        domain::price_components::FeeType::Rate { rates } => Fee::Rate {
            rates: rates.iter().map(term_rate_to_rest).collect(),
        },
        domain::price_components::FeeType::Slot {
            rates,
            slot_unit_name,
            minimum_count,
            quota,
            ..
        } => Fee::Slot {
            rates: rates.iter().map(term_rate_to_rest).collect(),
            slot_unit_name: slot_unit_name.clone(),
            minimum_count: *minimum_count,
            quota: *quota,
        },
        domain::price_components::FeeType::Capacity {
            metric_id,
            thresholds,
        } => Fee::Capacity {
            metric_id: *metric_id,
            thresholds: thresholds.iter().map(capacity_threshold_to_rest).collect(),
        },
        domain::price_components::FeeType::Usage { metric_id, pricing } => Fee::Usage {
            metric_id: *metric_id,
            pricing: usage_pricing_to_rest(pricing),
        },
        domain::price_components::FeeType::ExtraRecurring {
            unit_price,
            quantity,
            billing_type,
            cadence,
        } => Fee::ExtraRecurring {
            unit_price: *unit_price,
            quantity: *quantity,
            billing_type: billing_type.clone().into(),
            cadence: cadence.clone().into(),
        },
        domain::price_components::FeeType::OneTime {
            unit_price,
            quantity,
        } => Fee::OneTime {
            unit_price: *unit_price,
            quantity: *quantity,
        },
    }
}

fn term_rate_to_rest(rate: &domain::price_components::TermRate) -> TermRate {
    TermRate {
        term: rate.term.clone().into(),
        price: rate.price,
    }
}

fn capacity_threshold_to_rest(
    threshold: &domain::price_components::CapacityThreshold,
) -> CapacityThreshold {
    CapacityThreshold {
        included_amount: threshold.included_amount,
        price: threshold.price,
        per_unit_overage: threshold.per_unit_overage,
    }
}

fn usage_pricing_to_rest(
    pricing: &domain::price_components::UsagePricingModel,
) -> UsagePricingModel {
    match pricing {
        domain::price_components::UsagePricingModel::PerUnit { rate } => {
            UsagePricingModel::PerUnit { rate: *rate }
        }
        domain::price_components::UsagePricingModel::Tiered { tiers, block_size } => {
            UsagePricingModel::Tiered {
                tiers: tiers.iter().map(tier_row_to_rest).collect(),
                block_size: *block_size,
            }
        }
        domain::price_components::UsagePricingModel::Volume { tiers, block_size } => {
            UsagePricingModel::Volume {
                tiers: tiers.iter().map(tier_row_to_rest).collect(),
                block_size: *block_size,
            }
        }
        domain::price_components::UsagePricingModel::Package { block_size, rate } => {
            UsagePricingModel::Package {
                block_size: *block_size,
                rate: *rate,
            }
        }
        domain::price_components::UsagePricingModel::Matrix { rates } => {
            UsagePricingModel::Matrix {
                rates: rates.iter().map(matrix_row_to_rest).collect(),
            }
        }
    }
}

fn tier_row_to_rest(tier: &domain::price_components::TierRow) -> TierRow {
    TierRow {
        first_unit: tier.first_unit,
        rate: tier.rate,
        flat_fee: tier.flat_fee,
        flat_cap: tier.flat_cap,
    }
}

fn matrix_row_to_rest(row: &domain::price_components::MatrixRow) -> MatrixRow {
    MatrixRow {
        dimension1: MatrixDimension {
            key: row.dimension1.key.clone(),
            value: row.dimension1.value.clone(),
        },
        dimension2: row.dimension2.as_ref().map(|d| MatrixDimension {
            key: d.key.clone(),
            value: d.value.clone(),
        }),
        per_unit_price: row.per_unit_price,
    }
}

fn extract_available_parameters(
    price_components: &[domain::price_components::PriceComponent],
) -> AvailableParameters {
    let mut billing_periods: HashMap<String, Vec<BillingPeriodEnum>> = HashMap::new();
    let mut capacity_thresholds: HashMap<String, Vec<u64>> = HashMap::new();
    let mut slot_components: Vec<String> = Vec::new();

    for component in price_components {
        let component_id = component.id.to_string();

        match &component.fee {
            domain::price_components::FeeType::Rate { rates } if rates.len() > 1 => {
                let periods: Vec<BillingPeriodEnum> =
                    rates.iter().map(|r| r.term.clone().into()).collect();
                billing_periods.insert(component_id, periods);
            }
            domain::price_components::FeeType::Slot { rates, .. } => {
                if rates.len() > 1 {
                    let periods: Vec<BillingPeriodEnum> =
                        rates.iter().map(|r| r.term.clone().into()).collect();
                    billing_periods.insert(component_id.clone(), periods);
                }
                slot_components.push(component_id);
            }
            domain::price_components::FeeType::Capacity { thresholds, .. }
                if thresholds.len() > 1 =>
            {
                let values: Vec<u64> = thresholds.iter().map(|t| t.included_amount).collect();
                capacity_thresholds.insert(component_id, values);
            }
            _ => {}
        }
    }

    AvailableParameters {
        billing_periods,
        capacity_thresholds,
        slot_components,
    }
}
