use crate::currencies;
use crate::entitlements::mapping::entitlement_to_rest;
use crate::plans::model::{
    AvailableParameters, BillingPeriodEnum, CapacityPlanFee, CapacityThreshold, CreditExpiryUnit,
    CreditPackExpiry, CreditPackRollover, CreditPlanFee, ExtraRecurringPlanFee, Fee,
    MatrixPlanPricing, OneTimePlanFee, PackagePlanPricing, PerUnitPlanPricing, PercentagePricing,
    Plan, PlanFeeCreditUnit, PlanOption, PlanVersionSummary, PriceComponent, PriceComponentInput,
    ProductFamily, RateCardPlanPricing, RatePlanFee, SlotPlanFee, TermRate, TierRow,
    TieredPlanPricing, TrialConfig, UsagePlanFee, UsagePricingModel, VolumePlanPricing,
};
use meteroid_app::errors::RestApiError;
use meteroid_store::domain;
use meteroid_store::domain::Price;
use meteroid_store::domain::price_components::{
    PriceComponentNewInternal, PriceEntry, PriceInput, ProductRef,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use std::collections::HashMap;

// ── Domain → REST (response mapping) ──────────────────────────

pub fn plan_to_rest(
    plan: domain::Plan,
    version: domain::PlanVersion,
    price_components: Vec<domain::price_components::PriceComponent>,
    product_family_name: String,
    trial: Option<&domain::trials::PlanTrial>,
) -> Result<Plan, RestApiError> {
    let rest_components: Vec<PriceComponent> = price_components
        .iter()
        .map(price_component_to_rest)
        .collect();

    let available_parameters = extract_available_parameters(&price_components);

    let trial = trial.map(|trial| TrialConfig {
        duration_days: trial.offer.duration_days,
        is_free: trial.offer.free,
        trialing_plan_id: trial.offer.trial_plan_id,
    });

    let minimum_commitment = version
        .minimum
        .as_ref()
        .map(|m| plan_minimum_to_rest(m, &version.currency));

    let currency = currencies::mapping::from_str(&version.currency)?;

    Ok(Plan {
        id: plan.id,
        name: plan.name,
        description: plan.description,
        created_at: plan.created_at.into(),
        plan_type: plan.plan_type.into(),
        status: plan.status.into(),
        self_service_rank: plan.self_service_rank,
        issues_license: plan.issues_license,
        sold_for_resource_type_id: plan.sold_for_resource_type_id,
        product_family: ProductFamily {
            id: plan.product_family_id,
            name: product_family_name,
        },
        version_id: version.id,
        version: version.version,
        currency,
        tax_inclusive: version.tax_inclusive,
        net_terms: version.net_terms,
        billing_cycles: version.billing_cycles,
        period_start_day: version.period_start_day,
        trial,
        options: version
            .options
            .iter()
            .map(|option| PlanOption {
                id: option.id,
                name: option.name.clone(),
                alias: option.alias.clone(),
                hidden: option.hidden,
            })
            .collect(),
        price_components: rest_components,
        available_parameters,
        entitlements: version
            .entitlements
            .into_iter()
            .map(entitlement_to_rest)
            .collect(),
        minimum_commitment,
        credit_exhausted_rule: version
            .credit_exhausted_rule
            .as_ref()
            .map(credit_exhausted_rule_to_rest),
    })
}

pub fn credit_exhausted_rule_to_rest(
    rule: &domain::credit_exhausted::CreditExhaustedRule,
) -> crate::plans::model::CreditExhaustedRule {
    use crate::plans::model::{CreditCap, CreditCaps, CreditExhaustedRule};
    use domain::credit_exhausted::CreditExhaustedRule as Rule;
    match rule {
        Rule::Invoice => CreditExhaustedRule::Invoice,
        Rule::Pause => CreditExhaustedRule::Pause,
        Rule::InvoiceUpTo { caps } => CreditExhaustedRule::InvoiceUpTo(CreditCaps {
            caps: caps
                .iter()
                .map(|(denomination, amount)| CreditCap {
                    denomination: denomination.clone(),
                    amount: *amount,
                })
                .collect(),
        }),
    }
}

pub fn credit_exhausted_rule_from_rest(
    rule: crate::plans::model::CreditExhaustedRule,
) -> Result<domain::credit_exhausted::CreditExhaustedRule, RestApiError> {
    use crate::plans::model::CreditExhaustedRule;
    use domain::credit_exhausted::CreditExhaustedRule as Rule;
    match rule {
        CreditExhaustedRule::Invoice => Rule::Invoice,
        CreditExhaustedRule::Pause => Rule::Pause,
        CreditExhaustedRule::InvoiceUpTo(caps) => Rule::InvoiceUpTo {
            caps: caps
                .caps
                .into_iter()
                .map(|cap| (cap.denomination, cap.amount))
                .collect(),
        },
    }
    .validated()
    .map_err(|error| RestApiError::InvalidInput(error.to_string()))
}

pub fn plan_minimum_to_rest(
    min: &domain::plan_minimums::PlanMinimum,
    currency: &str,
) -> crate::plans::model::MinimumCommitment {
    minimum_commitment_to_rest(min.amount_subunit, &min.scope, currency)
}

pub fn subscription_minimum_to_rest(
    min: &domain::SubscriptionMinimum,
    currency: &str,
) -> crate::plans::model::MinimumCommitment {
    minimum_commitment_to_rest(min.amount_subunit, &min.scope, currency)
}

fn minimum_commitment_to_rest(
    amount_subunit: i64,
    scope: &domain::plan_minimums::MinimumScope,
    currency: &str,
) -> crate::plans::model::MinimumCommitment {
    use crate::plans::model::{
        AllComponentsScope, MinimumCommitment, MinimumCommitmentScope, ProductsScope,
    };
    use domain::plan_minimums::MinimumScope;
    let scope = match scope {
        MinimumScope::AllComponents => MinimumCommitmentScope::AllComponents(AllComponentsScope {}),
        MinimumScope::Products(ids) => MinimumCommitmentScope::Products(ProductsScope {
            product_ids: ids.iter().map(|id| id.to_string()).collect(),
        }),
    };
    MinimumCommitment {
        amount: subunit_to_major_string(amount_subunit, currency),
        scope,
    }
}

/// Render an i64 subunit amount as a major-unit decimal string for the wire format,
/// e.g. (5000, "USD") → "50.00", (5000, "JPY") → "5000", (50000000, "BTC") → "0.50000000".
/// Keeps trailing zeros to match the currency's precision.
fn subunit_to_major_string(amount_subunit: i64, currency: &str) -> String {
    use common_utils::decimals::ToUnit;
    use meteroid_store::constants::Currencies;
    let precision = Currencies::resolve_currency_precision(currency).unwrap_or(2);
    amount_subunit.to_unit(precision).to_string()
}

fn price_component_to_rest(component: &domain::price_components::PriceComponent) -> PriceComponent {
    let fee = if let Some(legacy) = &component.legacy_pricing {
        Some(fee_type_to_rest(&legacy.fee_type))
    } else {
        // The charge's own structure.
        component
            .fee_structure
            .as_ref()
            .and_then(|structure| v2_component_to_fee(structure, &component.prices))
    };

    PriceComponent {
        id: component.id,
        name: component.name.clone(),
        fee,
        product_id: component.product_id,
    }
}

/// Reconstruct a Fee from a v2 charge's FeeStructure + Prices (Pricing values).
fn v2_component_to_fee(structure: &FeeStructure, prices: &[Price]) -> Option<Fee> {
    if prices.is_empty() {
        return None;
    }

    match structure {
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
                    Pricing::Slot { model, .. } => Some(TermRate {
                        term: p.cadence.into(),
                        price: model.headline_rate(),
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
                credit_unit: prices.first().and_then(plan_fee_credit_unit),
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
                credit_unit: plan_fee_credit_unit(first),
            }))
        }
        FeeStructure::ExtraRecurring { billing_type } => {
            let first = prices.first()?;
            let cadence: BillingPeriodEnum = first.cadence.into();

            match &first.pricing {
                Pricing::ExtraRecurring {
                    unit_price,
                    quantity,
                    model,
                } => Some(Fee::ExtraRecurring(ExtraRecurringPlanFee {
                    unit_price: *unit_price,
                    quantity: *quantity,
                    billing_type: billing_type.clone().into(),
                    cadence,
                    model: fixed_quantity_model_to_rest(model.as_ref()),
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
                    model,
                } => Some(Fee::OneTime(OneTimePlanFee {
                    unit_price: *unit_price,
                    quantity: *quantity,
                    model: fixed_quantity_model_to_rest(model.as_ref()),
                })),
                _ => None,
            }
        }
        FeeStructure::Credit {} => {
            let first = prices.first()?;
            match &first.pricing {
                Pricing::Credit {
                    unit_price,
                    quantity,
                    credit_amount,
                    expiry,
                    rollover,
                    credit_unit_id,
                    once,
                } => Some(Fee::Credit(CreditPlanFee {
                    unit_price: *unit_price,
                    quantity: *quantity,
                    credit_amount: *credit_amount,
                    cadence: first.cadence.into(),
                    expiry: credit_expiry_to_rest(*expiry),
                    rollover: rollover.as_ref().map(credit_rollover_to_rest),
                    credit_unit_id: *credit_unit_id,
                    once: *once,
                })),
                _ => None,
            }
        }
    }
}

/// The model of an extra recurring or one-time price, when it has one.
fn fixed_quantity_model_to_rest(
    model: Option<&domain::price_components::RatingModel>,
) -> Option<UsagePricingModel> {
    model.map(|model| usage_pricing_to_rest(&model.as_usage_model()))
}

fn price_credit_unit(unit: &PlanFeeCreditUnit) -> domain::credit_units::PriceCreditUnit {
    domain::credit_units::PriceCreditUnit {
        unit_id: unit.credit_unit_id,
        rate: unit.rate,
    }
}

fn plan_fee_credit_unit(price: &Price) -> Option<PlanFeeCreditUnit> {
    price.credit_unit.as_ref().map(|unit| PlanFeeCreditUnit {
        credit_unit_id: unit.unit_id,
        rate: unit.rate,
    })
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
            credit_unit: None,
        }),
        domain::price_components::FeeType::Usage {
            metric_id,
            pricing,
            cadence,
        } => Fee::Usage(UsagePlanFee {
            metric_id: *metric_id,
            pricing: usage_pricing_to_rest(pricing),
            cadence: (*cadence).into(),
            credit_unit: None,
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
            model: None,
        }),
        domain::price_components::FeeType::OneTime {
            unit_price,
            quantity,
        } => Fee::OneTime(OneTimePlanFee {
            unit_price: *unit_price,
            quantity: *quantity,
            model: None,
        }),
        domain::price_components::FeeType::Credit {
            unit_price,
            quantity,
            credit_amount,
            expiry,
            once,
        } => Fee::Credit(CreditPlanFee {
            unit_price: *unit_price,
            quantity: *quantity,
            credit_amount: *credit_amount,
            cadence: BillingPeriodEnum::Monthly,
            expiry: credit_expiry_to_rest(*expiry),
            rollover: None,
            credit_unit_id: None,
            once: *once,
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
        // Rates a matrix can hold are returned as one, as before rates by dimension.
        domain::price_components::UsagePricingModel::DimensionRates(rates) => {
            match MatrixPlanPricing::from_domain(rates) {
                Some(matrix) => UsagePricingModel::Matrix(matrix),
                None => UsagePricingModel::DimensionRates(rates.clone().into()),
            }
        }
        domain::price_components::UsagePricingModel::RateCard {
            rate_card_id,
            multiplier,
        } => UsagePricingModel::RateCard(RateCardPlanPricing {
            rate_card_id: *rate_card_id,
            multiplier: *multiplier,
        }),
        domain::price_components::UsagePricingModel::Percentage { percent } => {
            UsagePricingModel::Percentage(PercentagePricing { percent: *percent })
        }
        domain::price_components::UsagePricingModel::Dynamic => UsagePricingModel::Dynamic,
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

pub fn plan_version_to_rest(v: &domain::PlanVersion) -> Result<PlanVersionSummary, RestApiError> {
    Ok(PlanVersionSummary {
        id: v.id,
        version: v.version,
        is_draft: v.is_draft_version,
        currency: currencies::mapping::from_str(&v.currency)?,
        created_at: v.created_at.into(),
    })
}

// ── REST → Domain (input mapping) ──────────────────────────────

pub fn rest_to_domain_component(
    input: &PriceComponentInput,
    currency: &str,
) -> Result<PriceComponentNewInternal, RestApiError> {
    let (fee_type_enum, fee_structure, prices) = fee_to_domain(&input.fee, currency)?;
    let fee_structure_of_charge = fee_structure.clone();

    let product_ref = match input.product_id {
        Some(pid) => ProductRef::Existing(pid),
        None => ProductRef::New {
            name: input.name.clone(),
            fee_type: fee_type_enum,
            fee_structure,
        },
    };

    // The fee's structure is the charge's (and a new product's for older readers).
    Ok(PriceComponentNewInternal {
        name: input.name.clone(),
        product_ref,
        fee_structure: Some(fee_structure_of_charge.clone()),
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
                        credit_unit: None,
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
                            model: meteroid_store::domain::price_components::RatingModel::PerUnit {
                                rate: r.price,
                            },
                            min_slots: f.minimum_count,
                            max_slots: f.quota,
                        },
                        credit_unit: None,
                    })
                })
                .collect();
            Ok((
                FeeTypeEnum::Slot,
                // Counts the quantity coded as its unit (REST v1 names no quantity).
                FeeStructure::Slot {
                    unit_name: f.slot_unit_name.clone(),
                    quantity_id: None,
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
                        credit_unit: f.credit_unit.as_ref().map(price_credit_unit),
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
                credit_unit: f.credit_unit.as_ref().map(price_credit_unit),
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
            let (unit_price, model) =
                fixed_quantity_pricing_from_rest(f.unit_price, f.model.as_ref())?;
            let prices = vec![PriceEntry::New(PriceInput {
                cadence: f.cadence.into(),
                currency: currency.to_string(),
                pricing: Pricing::ExtraRecurring {
                    unit_price,
                    quantity: f.quantity,
                    model,
                },
                credit_unit: None,
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
            let (unit_price, model) =
                fixed_quantity_pricing_from_rest(f.unit_price, f.model.as_ref())?;
            let prices = vec![PriceEntry::New(PriceInput {
                cadence: domain::enums::BillingPeriodEnum::Monthly,
                currency: currency.to_string(),
                pricing: Pricing::OneTime {
                    unit_price,
                    quantity: f.quantity,
                    model,
                },
                credit_unit: None,
            })];
            Ok((FeeTypeEnum::OneTime, FeeStructure::OneTime {}, prices))
        }
        Fee::Credit(f) => {
            let prices = vec![PriceEntry::New(PriceInput {
                cadence: f.cadence.into(),
                currency: currency.to_string(),
                pricing: Pricing::Credit {
                    unit_price: f.unit_price,
                    quantity: f.quantity,
                    credit_amount: f.credit_amount,
                    expiry: credit_expiry_to_domain(f.expiry)?,
                    credit_unit_id: f.credit_unit_id,
                    rollover: f
                        .rollover
                        .clone()
                        .map(credit_rollover_to_domain)
                        .transpose()?,
                    once: f.once,
                },
                credit_unit: None,
            })];
            Ok((FeeTypeEnum::Credit, FeeStructure::Credit {}, prices))
        }
    }
}

pub fn credit_expiry_to_domain(
    expiry: Option<CreditPackExpiry>,
) -> Result<Option<domain::price_components::CreditExpiry>, RestApiError> {
    use domain::price_components::CreditExpiry;
    let Some(expiry) = expiry else {
        return Ok(None);
    };
    match (expiry.unit, expiry.value) {
        (_, 0) => Err(RestApiError::InvalidInput(
            "credit expiry value must be positive".into(),
        )),
        (CreditExpiryUnit::Day, days) => Ok(Some(CreditExpiry::Day(days))),
        (CreditExpiryUnit::Month, months) => Ok(Some(CreditExpiry::Month(months))),
        (CreditExpiryUnit::BillingPeriod, 1) => Ok(Some(CreditExpiry::BillingPeriod)),
        (CreditExpiryUnit::BillingPeriod, _) => Err(RestApiError::InvalidInput(
            "billing-period credit expiry takes value 1".into(),
        )),
    }
}

fn credit_rollover_to_domain(
    rollover: CreditPackRollover,
) -> Result<domain::price_components::CreditRollover, RestApiError> {
    Ok(domain::price_components::CreditRollover {
        max_amount: rollover.max_amount,
        expiry: credit_expiry_to_domain(Some(rollover.expiry))?
            .ok_or_else(|| RestApiError::InvalidInput("a rollover needs a term".into()))?,
    })
}

pub fn credit_rollover_to_rest(
    rollover: &domain::price_components::CreditRollover,
) -> CreditPackRollover {
    CreditPackRollover {
        max_amount: rollover.max_amount,
        expiry: credit_term_to_rest(rollover.expiry),
    }
}

pub fn credit_expiry_to_rest(
    expiry: Option<domain::price_components::CreditExpiry>,
) -> Option<CreditPackExpiry> {
    expiry.map(credit_term_to_rest)
}

fn credit_term_to_rest(expiry: domain::price_components::CreditExpiry) -> CreditPackExpiry {
    use domain::price_components::CreditExpiry;
    match expiry {
        CreditExpiry::Day(value) => CreditPackExpiry {
            unit: CreditExpiryUnit::Day,
            value,
        },
        CreditExpiry::Month(value) => CreditPackExpiry {
            unit: CreditExpiryUnit::Month,
            value,
        },
        CreditExpiry::BillingPeriod => CreditPackExpiry {
            unit: CreditExpiryUnit::BillingPeriod,
            value: 1,
        },
    }
}

/// A model a count (seats, a quantity set on the plan) can be priced by, through the seat
/// validator; a matrix, a rate card, tiers in blocks or an empty package is a 400.
pub fn rating_model_from_rest(
    model: domain::price_components::UsagePricingModel,
) -> Result<domain::price_components::RatingModel, RestApiError> {
    let model = domain::price_components::RatingModel::from_usage_model(model)
        .map_err(|e| RestApiError::InvalidInput(e.to_string()))?;
    // A model's checks read no currency.
    domain::prices::validate_pricing(
        &Pricing::Slot {
            model: model.clone(),
            min_slots: None,
            max_slots: None,
        },
        "",
    )
    .map_err(|e| RestApiError::InvalidInput(e.to_string()))?;
    Ok(model)
}

/// The unit price and model of an extra recurring or one-time fee, stored as gRPC stores them:
/// a per-unit model becomes the unit price; any other leaves its first rate there.
fn fixed_quantity_pricing_from_rest(
    unit_price: rust_decimal::Decimal,
    model: Option<&UsagePricingModel>,
) -> Result<
    (
        rust_decimal::Decimal,
        Option<domain::price_components::RatingModel>,
    ),
    RestApiError,
> {
    use domain::price_components::RatingModel;
    let Some(model) = model else {
        return Ok((unit_price, None));
    };
    match rating_model_from_rest(rest_usage_pricing_to_domain(model))? {
        RatingModel::PerUnit { rate } => Ok((rate, None)),
        model => Ok((model.headline_rate(), Some(model))),
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
        UsagePricingModel::Matrix(p) => D::DimensionRates(p.to_domain()),
        UsagePricingModel::DimensionRates(p) => D::DimensionRates(p.clone().into()),
        UsagePricingModel::RateCard(p) => D::RateCard {
            rate_card_id: p.rate_card_id,
            multiplier: p.multiplier,
        },
        UsagePricingModel::Percentage(p) => D::Percentage { percent: p.percent },
        UsagePricingModel::Dynamic => D::Dynamic,
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

// ── Available parameters extraction ────────────────────────────

fn extract_available_parameters(
    price_components: &[domain::price_components::PriceComponent],
) -> AvailableParameters {
    use domain::enums::FeeTypeEnum;
    use domain::prices::Pricing;

    let mut billing_periods: HashMap<String, Vec<BillingPeriodEnum>> = HashMap::new();
    let mut capacity_thresholds: HashMap<String, Vec<u64>> = HashMap::new();
    let mut slot_components: Vec<String> = Vec::new();

    for component in price_components {
        let component_id = component.id.to_string();

        if !component.prices.is_empty() {
            let fee_type = component.fee_structure.as_ref().map(|fs| fs.fee_type());

            let is_slot = fee_type == Some(FeeTypeEnum::Slot);
            let is_capacity = fee_type == Some(FeeTypeEnum::Capacity);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plans::model::{BillingType, MatrixDimension, MatrixPlanPricing, MatrixRow};
    use rust_decimal::Decimal;

    fn package_fee(model: Option<UsagePricingModel>) -> Fee {
        Fee::ExtraRecurring(ExtraRecurringPlanFee {
            unit_price: Decimal::from(25),
            quantity: 25,
            billing_type: BillingType::Advance,
            cadence: BillingPeriodEnum::Monthly,
            model,
        })
    }

    fn package() -> UsagePricingModel {
        UsagePricingModel::Package(PackagePlanPricing {
            block_size: 10,
            rate: Decimal::from(30),
        })
    }

    fn written(fee: &Fee) -> (FeeStructure, Pricing) {
        let (_, structure, mut prices) = fee_to_domain(fee, "EUR").expect("a valid fee");
        match prices.pop() {
            Some(PriceEntry::New(input)) => (structure, input.pricing),
            _ => panic!("one new price"),
        }
    }

    fn read(structure: FeeStructure, pricing: Pricing) -> Fee {
        let price = Price {
            id: Default::default(),
            product_id: common_domain::ids::ProductId::default(),
            cadence: domain::enums::BillingPeriodEnum::Monthly,
            currency: "EUR".into(),
            pricing,
            tenant_id: Default::default(),
            created_at: chrono::NaiveDateTime::default(),
            archived_at: None,
            catalog: false,
            credit_unit: None,
        };
        v2_component_to_fee(&structure, &[price]).expect("a fee")
    }

    fn billed(pricing: &Pricing) -> Decimal {
        match pricing {
            Pricing::ExtraRecurring {
                unit_price,
                quantity,
                model,
            } => domain::price_components::fixed_quantity_amount(
                *unit_price,
                Decimal::from(*quantity),
                model.as_ref(),
            ),
            _ => panic!("an extra recurring price"),
        }
    }

    #[test]
    fn a_package_fee_reads_and_writes_back_as_billed() {
        // 25 units in packs of 10 at 30: 3 packs, 90, never 25 × 25.
        let (structure, pricing) = written(&package_fee(Some(package())));
        assert_eq!(billed(&pricing), Decimal::from(90));

        let Fee::ExtraRecurring(read_back) = read(structure, pricing) else {
            panic!("an extra recurring fee");
        };
        assert_eq!(read_back.unit_price, Decimal::from(30));
        assert!(matches!(
            &read_back.model,
            Some(UsagePricingModel::Package(p)) if p.block_size == 10 && p.rate == Decimal::from(30)
        ));

        let (_, rewritten) = written(&Fee::ExtraRecurring(read_back));
        assert_eq!(billed(&rewritten), Decimal::from(90));
    }

    #[test]
    fn a_fee_without_a_model_keeps_its_unit_price() {
        let (structure, pricing) = written(&package_fee(None));
        assert_eq!(billed(&pricing), Decimal::from(625));
        let Fee::ExtraRecurring(read_back) = read(structure, pricing) else {
            panic!("an extra recurring fee");
        };
        assert!(read_back.model.is_none());
    }

    #[test]
    fn a_model_a_count_cannot_take_is_refused() {
        let matrix = UsagePricingModel::Matrix(MatrixPlanPricing { rates: vec![] });
        assert!(fee_to_domain(&package_fee(Some(matrix)), "EUR").is_err());
        let blocks = UsagePricingModel::Tiered(TieredPlanPricing {
            tiers: vec![],
            block_size: Some(5),
        });
        assert!(fee_to_domain(&package_fee(Some(blocks)), "EUR").is_err());
        let empty = UsagePricingModel::Package(PackagePlanPricing {
            block_size: 0,
            rate: Decimal::from(30),
        });
        assert!(fee_to_domain(&package_fee(Some(empty)), "EUR").is_err());
    }

    /// A matrix sent to the API is written as rates by dimension of its exact values, and
    /// returned as the matrix it was.
    #[test]
    fn a_matrix_is_written_as_rates_by_dimension() {
        let dimension = |key: &str, value: &str| MatrixDimension {
            key: key.to_owned(),
            value: value.to_owned(),
        };
        let fee = Fee::Usage(UsagePlanFee {
            metric_id: common_domain::ids::BillableMetricId::default(),
            pricing: UsagePricingModel::Matrix(MatrixPlanPricing {
                rates: vec![MatrixRow {
                    dimension1: dimension("region", "eu"),
                    dimension2: Some(dimension("tier", "gold")),
                    per_unit_price: Decimal::from(2),
                }],
            }),
            cadence: BillingPeriodEnum::Monthly,
            credit_unit: None,
        });
        let (_, structure, mut prices) = fee_to_domain(&fee, "EUR").expect("a valid fee");
        assert!(matches!(
            structure,
            FeeStructure::Usage {
                model: UsageModel::DimensionRates,
                ..
            }
        ));
        let Some(PriceEntry::New(input)) = prices.pop() else {
            panic!("one new price");
        };
        let Pricing::Usage(domain::price_components::UsagePricingModel::DimensionRates(rates)) =
            input.pricing
        else {
            panic!("rates by dimension");
        };
        assert_eq!(rates.dimensions, vec!["region", "tier"]);
        assert_eq!(rates.rates[0].key, vec!["eu", "gold"]);
        assert_eq!(rates.rates[0].rate, Decimal::from(2));
        let UsagePricingModel::Matrix(matrix) = usage_pricing_to_rest(
            &domain::price_components::UsagePricingModel::DimensionRates(rates.clone()),
        ) else {
            panic!("returned as a matrix");
        };
        let [row] = matrix.rates.as_slice() else {
            panic!("one cell");
        };
        assert_eq!(
            (
                (row.dimension1.key.as_str(), row.dimension1.value.as_str()),
                row.dimension2
                    .as_ref()
                    .map(|cell| (cell.key.as_str(), cell.value.as_str())),
                row.per_unit_price,
            ),
            (("region", "eu"), Some(("tier", "gold")), Decimal::from(2))
        );

        // A prefix is not a matrix's: returned as rates by dimension.
        let mut prefixed = rates;
        prefixed.rates[0].key[1] = "go*".to_owned();
        assert!(matches!(
            usage_pricing_to_rest(
                &domain::price_components::UsagePricingModel::DimensionRates(prefixed)
            ),
            UsagePricingModel::DimensionRates(_)
        ));
    }
}
