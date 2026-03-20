use crate::api_rest::plans::model::{
    AvailableParameters, BillingPeriodEnum, CapacityPlanFee, CapacityThreshold,
    ExtraRecurringPlanFee, Fee, MatrixDimension, MatrixPlanPricing, MatrixRow, OneTimePlanFee,
    PackagePlanPricing, PerUnitPlanPricing, Plan, PlanVersionSummary, PriceComponent,
    PriceComponentInput, ProductFamily, RatePlanFee, SlotPlanFee, TermRate, TierRow,
    TieredPlanPricing, TrialConfig, UsagePlanFee, UsagePricingModel, VolumePlanPricing,
};
use crate::errors::RestApiError;
use common_domain::ids::ProductId;
use meteroid_store::domain;
use meteroid_store::domain::Price;
use meteroid_store::domain::price_components::{
    PriceComponentNewInternal, PriceEntry, PriceInput, ProductRef,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::products::Product;
use std::collections::HashMap;

// ── Domain → REST (response mapping) ──────────────────────────

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

    let trial = version.trial_duration_days.map(|d| TrialConfig {
        duration_days: d as u32,
        is_free: version.trial_is_free,
        trialing_plan_id: version.trialing_plan_id,
    });

    Plan {
        id: plan.id,
        name: plan.name,
        description: plan.description,
        created_at: plan.created_at,
        plan_type: plan.plan_type.into(),
        status: plan.status.into(),
        self_service_rank: plan.self_service_rank,
        product_family: ProductFamily {
            id: plan.product_family_id,
            name: product_family_name,
        },
        version_id: version.id,
        version: version.version,
        currency: version.currency,
        net_terms: version.net_terms,
        billing_cycles: version.billing_cycles,
        period_start_day: version.period_start_day,
        trial,
        price_components: rest_components,
        available_parameters,
    }
}

fn price_component_to_rest(
    component: &domain::price_components::PriceComponent,
    products: &HashMap<ProductId, Product>,
) -> PriceComponent {
    let fee = if let Some(legacy) = &component.legacy_pricing {
        Some(fee_type_to_rest(&legacy.fee_type))
    } else if let Some(product_id) = component.product_id {
        products
            .get(&product_id)
            .and_then(|product| v2_component_to_fee(product, &component.prices))
    } else {
        None
    };

    PriceComponent {
        id: component.id,
        name: component.name.clone(),
        fee,
        product_id: component.product_id,
    }
}

/// Reconstruct a Fee from a v2 Product (FeeStructure) + Prices (Pricing values).
fn v2_component_to_fee(product: &Product, prices: &[Price]) -> Option<Fee> {
    if prices.is_empty() {
        return None;
    }

    match &product.fee_structure {
        FeeStructure::Rate {} => {
            let rates = prices
                .iter()
                .filter_map(|p| match &p.pricing {
                    Pricing::Rate { rate } => Some(TermRate {
                        term: p.cadence.into(),
                        price: *rate,
                    }),
                    _ => None,
                })
                .collect();
            Some(Fee::Rate(RatePlanFee { rates }))
        }
        FeeStructure::Slot { unit_name, .. } => {
            let first_pricing = prices.first().and_then(|p| match &p.pricing {
                Pricing::Slot {
                    min_slots,
                    max_slots,
                    ..
                } => Some((*min_slots, *max_slots)),
                _ => None,
            });
            let (minimum_count, quota) = first_pricing.unwrap_or((None, None));

            let rates = prices
                .iter()
                .filter_map(|p| match &p.pricing {
                    Pricing::Slot { unit_rate, .. } => Some(TermRate {
                        term: p.cadence.into(),
                        price: *unit_rate,
                    }),
                    _ => None,
                })
                .collect();

            Some(Fee::Slot(SlotPlanFee {
                rates,
                slot_unit_name: unit_name.clone(),
                minimum_count,
                quota,
            }))
        }
        FeeStructure::Capacity { metric_id } => {
            let cadence = prices
                .first()
                .map(|p| p.cadence.into())
                .unwrap_or(BillingPeriodEnum::Monthly);

            let thresholds = prices
                .iter()
                .filter_map(|p| match &p.pricing {
                    Pricing::Capacity {
                        rate,
                        included,
                        overage_rate,
                    } => Some(CapacityThreshold {
                        included_amount: *included,
                        price: *rate,
                        per_unit_overage: *overage_rate,
                    }),
                    _ => None,
                })
                .collect();

            Some(Fee::Capacity(CapacityPlanFee {
                metric_id: *metric_id,
                thresholds,
                cadence,
            }))
        }
        FeeStructure::Usage { metric_id, .. } => {
            let first = prices.first()?;
            let cadence: BillingPeriodEnum = first.cadence.into();

            let pricing = match &first.pricing {
                Pricing::Usage(model) => Some(usage_pricing_to_rest(model)),
                _ => None,
            }?;

            Some(Fee::Usage(UsagePlanFee {
                metric_id: *metric_id,
                pricing,
                cadence,
            }))
        }
        FeeStructure::ExtraRecurring { billing_type } => {
            let first = prices.first()?;
            let cadence: BillingPeriodEnum = first.cadence.into();

            match &first.pricing {
                Pricing::ExtraRecurring {
                    unit_price,
                    quantity,
                } => Some(Fee::ExtraRecurring(ExtraRecurringPlanFee {
                    unit_price: *unit_price,
                    quantity: *quantity,
                    billing_type: billing_type.clone().into(),
                    cadence,
                })),
                _ => None,
            }
        }
        FeeStructure::OneTime {} => {
            let first = prices.first()?;
            match &first.pricing {
                Pricing::OneTime {
                    unit_price,
                    quantity,
                } => Some(Fee::OneTime(OneTimePlanFee {
                    unit_price: *unit_price,
                    quantity: *quantity,
                })),
                _ => None,
            }
        }
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

pub fn plan_version_to_rest(v: &domain::PlanVersion) -> PlanVersionSummary {
    PlanVersionSummary {
        id: v.id,
        version: v.version,
        is_draft: v.is_draft_version,
        currency: v.currency.clone(),
        created_at: v.created_at,
    }
}

// ── REST → Domain (input mapping) ──────────────────────────────

pub fn rest_to_domain_component(
    input: &PriceComponentInput,
    currency: &str,
) -> Result<PriceComponentNewInternal, RestApiError> {
    let (fee_type_enum, fee_structure, prices) = fee_to_domain(&input.fee, currency)?;

    let product_ref = match input.product_id {
        Some(pid) => ProductRef::Existing(pid),
        None => ProductRef::New {
            name: input.name.clone(),
            fee_type: fee_type_enum,
            fee_structure,
        },
    };

    Ok(PriceComponentNewInternal {
        name: input.name.clone(),
        product_ref,
        prices,
    })
}

fn fee_to_domain(
    fee: &Fee,
    currency: &str,
) -> Result<(domain::enums::FeeTypeEnum, FeeStructure, Vec<PriceEntry>), RestApiError> {
    use domain::enums::FeeTypeEnum;
    use domain::price_components::{DowngradePolicy, UpgradePolicy};

    match fee {
        Fee::Rate(f) => {
            if f.rates.is_empty() {
                return Err(RestApiError::InvalidInput(
                    "Rate fee must have at least one rate".into(),
                ));
            }
            let prices = f
                .rates
                .iter()
                .map(|r| {
                    PriceEntry::New(PriceInput {
                        cadence: r.term.into(),
                        currency: currency.to_string(),
                        pricing: Pricing::Rate { rate: r.price },
                    })
                })
                .collect();
            Ok((FeeTypeEnum::Rate, FeeStructure::Rate {}, prices))
        }
        Fee::Slot(f) => {
            if f.rates.is_empty() {
                return Err(RestApiError::InvalidInput(
                    "Slot fee must have at least one rate".into(),
                ));
            }
            let prices = f
                .rates
                .iter()
                .map(|r| {
                    PriceEntry::New(PriceInput {
                        cadence: r.term.into(),
                        currency: currency.to_string(),
                        pricing: Pricing::Slot {
                            unit_rate: r.price,
                            min_slots: f.minimum_count,
                            max_slots: f.quota,
                        },
                    })
                })
                .collect();
            Ok((
                FeeTypeEnum::Slot,
                FeeStructure::Slot {
                    unit_name: f.slot_unit_name.clone(),
                    upgrade_policy: UpgradePolicy::Prorated,
                    downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
                },
                prices,
            ))
        }
        Fee::Capacity(f) => {
            if f.thresholds.is_empty() {
                return Err(RestApiError::InvalidInput(
                    "Capacity fee must have at least one threshold".into(),
                ));
            }
            let prices = f
                .thresholds
                .iter()
                .map(|t| {
                    PriceEntry::New(PriceInput {
                        cadence: f.cadence.into(),
                        currency: currency.to_string(),
                        pricing: Pricing::Capacity {
                            rate: t.price,
                            included: t.included_amount,
                            overage_rate: t.per_unit_overage,
                        },
                    })
                })
                .collect();
            Ok((
                FeeTypeEnum::Capacity,
                FeeStructure::Capacity {
                    metric_id: f.metric_id,
                },
                prices,
            ))
        }
        Fee::Usage(f) => {
            let domain_model = rest_usage_pricing_to_domain(&f.pricing);
            let usage_model = UsageModel::from(&domain_model);
            let prices = vec![PriceEntry::New(PriceInput {
                cadence: f.cadence.into(),
                currency: currency.to_string(),
                pricing: Pricing::Usage(domain_model),
            })];
            Ok((
                FeeTypeEnum::Usage,
                FeeStructure::Usage {
                    metric_id: f.metric_id,
                    model: usage_model,
                },
                prices,
            ))
        }
        Fee::ExtraRecurring(f) => {
            let prices = vec![PriceEntry::New(PriceInput {
                cadence: f.cadence.into(),
                currency: currency.to_string(),
                pricing: Pricing::ExtraRecurring {
                    unit_price: f.unit_price,
                    quantity: f.quantity,
                },
            })];
            Ok((
                FeeTypeEnum::ExtraRecurring,
                FeeStructure::ExtraRecurring {
                    billing_type: f.billing_type.into(),
                },
                prices,
            ))
        }
        Fee::OneTime(f) => {
            // OneTime fees have no meaningful cadence, but the price table requires one.
            // Monthly is used as a storage placeholder; billing logic uses
            // SubscriptionFeeBillingPeriod::OneTime for actual scheduling.
            let prices = vec![PriceEntry::New(PriceInput {
                cadence: domain::enums::BillingPeriodEnum::Monthly,
                currency: currency.to_string(),
                pricing: Pricing::OneTime {
                    unit_price: f.unit_price,
                    quantity: f.quantity,
                },
            })];
            Ok((FeeTypeEnum::OneTime, FeeStructure::OneTime {}, prices))
        }
    }
}

fn rest_usage_pricing_to_domain(
    model: &UsagePricingModel,
) -> domain::price_components::UsagePricingModel {
    use domain::price_components::UsagePricingModel as D;

    match model {
        UsagePricingModel::PerUnit(p) => D::PerUnit { rate: p.rate },
        UsagePricingModel::Tiered(p) => D::Tiered {
            tiers: p.tiers.iter().map(tier_row_to_domain).collect(),
            block_size: p.block_size,
        },
        UsagePricingModel::Volume(p) => D::Volume {
            tiers: p.tiers.iter().map(tier_row_to_domain).collect(),
            block_size: p.block_size,
        },
        UsagePricingModel::Package(p) => D::Package {
            block_size: p.block_size,
            rate: p.rate,
        },
        UsagePricingModel::Matrix(p) => D::Matrix {
            rates: p.rates.iter().map(matrix_row_to_domain).collect(),
        },
    }
}

fn tier_row_to_domain(t: &TierRow) -> domain::price_components::TierRow {
    domain::price_components::TierRow {
        first_unit: t.first_unit,
        rate: t.rate,
        flat_fee: t.flat_fee,
        flat_cap: t.flat_cap,
    }
}

fn matrix_row_to_domain(r: &MatrixRow) -> domain::price_components::MatrixRow {
    domain::price_components::MatrixRow {
        dimension1: domain::price_components::MatrixDimension {
            key: r.dimension1.key.clone(),
            value: r.dimension1.value.clone(),
        },
        dimension2: r
            .dimension2
            .as_ref()
            .map(|d| domain::price_components::MatrixDimension {
                key: d.key.clone(),
                value: d.value.clone(),
            }),
        per_unit_price: r.per_unit_price,
    }
}

// ── Available parameters extraction ────────────────────────────

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
