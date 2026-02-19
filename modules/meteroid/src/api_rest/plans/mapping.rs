use crate::api_rest::plans::model::{
    AvailableParameters, BillingPeriodEnum, CapacityPlanFee, CapacityThreshold,
    ExtraRecurringPlanFee, Fee, MatrixDimension, MatrixPlanPricing, MatrixRow, OneTimePlanFee,
    PackagePlanPricing, PerUnitPlanPricing, Plan, PriceComponent, ProductFamily, RatePlanFee,
    SlotPlanFee, TermRate, TierRow, TieredPlanPricing, TrialConfig, UsagePlanFee,
    UsagePricingModel, VolumePlanPricing,
};
use common_domain::ids::ProductId;
use meteroid_store::domain;
use meteroid_store::domain::products::Product;
use std::collections::HashMap;

pub fn plan_to_rest(
    plan: domain::Plan,
    version: domain::PlanVersion,
    price_components: Vec<domain::price_components::PriceComponent>,
    product_family_name: String,
    products: &HashMap<ProductId, Product>,
) -> Plan {
    let rest_components: Vec<PriceComponent> = price_components
        .iter()
        .map(|c| price_component_to_rest(c, products))
        .collect();

    let available_parameters = extract_available_parameters(&price_components, products);

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

fn price_component_to_rest(
    component: &domain::price_components::PriceComponent,
    _products: &HashMap<ProductId, Product>,
) -> PriceComponent {
    // For v1 components, populate fee from legacy_pricing for REST backward compat
    let fee = component
        .legacy_pricing
        .as_ref()
        .map(|legacy| fee_type_to_rest(&legacy.fee_type));

    PriceComponent {
        id: component.id,
        name: component.name.clone(),
        fee,
        product_id: component.product_id,
    }
}

fn fee_type_to_rest(fee: &domain::price_components::FeeType) -> Fee {
    match fee {
        domain::price_components::FeeType::Rate { rates } => Fee::Rate(RatePlanFee {
            rates: rates.iter().map(term_rate_to_rest).collect(),
        }),
        domain::price_components::FeeType::Slot {
            rates,
            slot_unit_name,
            minimum_count,
            quota,
            ..
        } => Fee::Slot(SlotPlanFee {
            rates: rates.iter().map(term_rate_to_rest).collect(),
            slot_unit_name: slot_unit_name.clone(),
            minimum_count: *minimum_count,
            quota: *quota,
        }),
        domain::price_components::FeeType::Capacity {
            metric_id,
            thresholds,
            cadence,
        } => Fee::Capacity(CapacityPlanFee {
            metric_id: *metric_id,
            thresholds: thresholds.iter().map(capacity_threshold_to_rest).collect(),
            cadence: (*cadence).into(),
        }),
        domain::price_components::FeeType::Usage {
            metric_id,
            pricing,
            cadence,
        } => Fee::Usage(UsagePlanFee {
            metric_id: *metric_id,
            pricing: usage_pricing_to_rest(pricing),
            cadence: (*cadence).into(),
        }),
        domain::price_components::FeeType::ExtraRecurring {
            unit_price,
            quantity,
            billing_type,
            cadence,
        } => Fee::ExtraRecurring(ExtraRecurringPlanFee {
            unit_price: *unit_price,
            quantity: *quantity,
            billing_type: billing_type.clone().into(),
            cadence: (*cadence).into(),
        }),
        domain::price_components::FeeType::OneTime {
            unit_price,
            quantity,
        } => Fee::OneTime(OneTimePlanFee {
            unit_price: *unit_price,
            quantity: *quantity,
        }),
    }
}

fn term_rate_to_rest(rate: &domain::price_components::TermRate) -> TermRate {
    TermRate {
        term: rate.term.into(),
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
            UsagePricingModel::PerUnit(PerUnitPlanPricing { rate: *rate })
        }
        domain::price_components::UsagePricingModel::Tiered { tiers, block_size } => {
            UsagePricingModel::Tiered(TieredPlanPricing {
                tiers: tiers.iter().map(tier_row_to_rest).collect(),
                block_size: *block_size,
            })
        }
        domain::price_components::UsagePricingModel::Volume { tiers, block_size } => {
            UsagePricingModel::Volume(VolumePlanPricing {
                tiers: tiers.iter().map(tier_row_to_rest).collect(),
                block_size: *block_size,
            })
        }
        domain::price_components::UsagePricingModel::Package { block_size, rate } => {
            UsagePricingModel::Package(PackagePlanPricing {
                block_size: *block_size,
                rate: *rate,
            })
        }
        domain::price_components::UsagePricingModel::Matrix { rates } => {
            UsagePricingModel::Matrix(MatrixPlanPricing {
                rates: rates.iter().map(matrix_row_to_rest).collect(),
            })
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
    products: &HashMap<ProductId, Product>,
) -> AvailableParameters {
    use domain::enums::FeeTypeEnum;
    use domain::prices::Pricing;

    let mut billing_periods: HashMap<String, Vec<BillingPeriodEnum>> = HashMap::new();
    let mut capacity_thresholds: HashMap<String, Vec<u64>> = HashMap::new();
    let mut slot_components: Vec<String> = Vec::new();

    for component in price_components {
        let component_id = component.id.to_string();

        // For v2 components, use prices. For v1, use legacy_pricing entries.
        if !component.prices.is_empty() {
            let fee_type = component
                .product_id
                .and_then(|pid| products.get(&pid))
                .map(|p| &p.fee_type);

            let is_slot = fee_type == Some(&FeeTypeEnum::Slot);
            let is_capacity = fee_type == Some(&FeeTypeEnum::Capacity);

            if component.prices.len() > 1 || is_slot {
                let cadences: Vec<BillingPeriodEnum> = component
                    .prices
                    .iter()
                    .map(|p| p.cadence.into())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();

                if cadences.len() > 1 {
                    billing_periods.insert(component_id.clone(), cadences);
                }
            }

            if is_slot {
                slot_components.push(component_id);
            } else if is_capacity && component.prices.len() > 1 {
                let values: Vec<u64> = component
                    .prices
                    .iter()
                    .filter_map(|p| match &p.pricing {
                        Pricing::Capacity { included, .. } => Some(*included),
                        _ => None,
                    })
                    .collect();
                if values.len() > 1 {
                    capacity_thresholds.insert(component_id, values);
                }
            }
        } else if let Some(legacy) = &component.legacy_pricing {
            // V1 legacy path: derive parameters from legacy pricing entries
            let is_slot = matches!(
                legacy.fee_type,
                domain::price_components::FeeType::Slot { .. }
            );
            let is_capacity = matches!(
                legacy.fee_type,
                domain::price_components::FeeType::Capacity { .. }
            );

            if legacy.pricing_entries.len() > 1 || is_slot {
                let cadences: Vec<BillingPeriodEnum> = legacy
                    .pricing_entries
                    .iter()
                    .map(|(c, _)| (*c).into())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();

                if cadences.len() > 1 {
                    billing_periods.insert(component_id.clone(), cadences);
                }
            }

            if is_slot {
                slot_components.push(component_id);
            } else if is_capacity && legacy.pricing_entries.len() > 1 {
                let values: Vec<u64> = legacy
                    .pricing_entries
                    .iter()
                    .filter_map(|(_, p)| match p {
                        Pricing::Capacity { included, .. } => Some(*included),
                        _ => None,
                    })
                    .collect();
                if values.len() > 1 {
                    capacity_thresholds.insert(component_id, values);
                }
            }
        }
    }

    AvailableParameters {
        billing_periods,
        capacity_thresholds,
        slot_components,
    }
}
