use crate::TaxEngineError;
use crate::model::{
    Address, CalculationResult, CustomerTax, LineItemForTax, LineItemWithTax, TaxBreakdownItem,
    TaxDetails, TaxRule, VatExemptionReason,
};
use error_stack::Report;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

pub(crate) async fn compute_tax(
    customer_tax: CustomerTax,
    invoicing_entity_address: Address,
    line_items: Vec<LineItemForTax>,
) -> Result<Vec<LineItemWithTax>, Report<TaxEngineError>> {
    // Check for customer-level exemptions
    let customer_exemption = match &customer_tax {
        CustomerTax::ResolvedTaxRate(world_tax::TaxRate {
            tax_type: world_tax::TaxType::VAT(world_tax::VatRate::Exempt),
            ..
        }) => Some(VatExemptionReason::TaxExempt),
        CustomerTax::ResolvedTaxRate(world_tax::TaxRate {
            tax_type: world_tax::TaxType::VAT(world_tax::VatRate::ReverseCharge),
            ..
        }) => Some(VatExemptionReason::ReverseCharge),
        CustomerTax::Exempt => Some(VatExemptionReason::TaxExempt),
        _ => None,
    };

    if let Some(exemption) = customer_exemption {
        // If the customer is exempt, we do not apply any tax
        let computed_line_items: Vec<LineItemWithTax> = line_items
            .into_iter()
            .map(|item| LineItemWithTax {
                line_id: item.line_id,
                pre_tax_amount: item.amount,
                tax_details: TaxDetails::Exempt(exemption.clone()),
            })
            .collect();

        return Ok(computed_line_items);
    }

    let mut computed_line_items = Vec::new();
    for item in line_items {
        let tax_details = determine_tax_details(&item, &customer_tax, &invoicing_entity_address);

        computed_line_items.push(LineItemWithTax {
            line_id: item.line_id,
            pre_tax_amount: item.amount,
            tax_details,
        });
    }

    Ok(computed_line_items)
}

fn determine_tax_details(
    item: &LineItemForTax,
    customer_tax: &CustomerTax,
    invoicing_entity_address: &Address,
) -> TaxDetails {
    use crate::model::TaxItem;

    let invoicing_entity_country = match &invoicing_entity_address.country {
        Some(country) => country,
        None => return TaxDetails::Exempt(VatExemptionReason::NotRegistered),
    };

    // First check custom taxes on line item (product-level)
    if !item.custom_taxes.is_empty() {
        let mut taxes = Vec::new();
        let mut total_tax_amount = 0u64;

        for custom_tax in &item.custom_taxes {
            // Find the most specific applicable rule for this tax
            let mut applicable_rules: Vec<&TaxRule> = custom_tax
                .tax_rules
                .iter()
                .filter(|rule| {
                    let mut include = true;

                    if let Some(country) = &rule.country {
                        include = country == invoicing_entity_country;
                    }

                    if let Some(region) = &rule.region {
                        if let Some(ie_region) = &invoicing_entity_address.region {
                            include = include && region == ie_region;
                        } else {
                            include = false;
                        }
                    }

                    include
                })
                .collect();

            // Sort by specificity (region > country > none)
            applicable_rules.sort_by(|a, b| {
                fn priority(rule: &TaxRule) -> i32 {
                    match (&rule.region, &rule.country) {
                        (Some(_), _) => 2,    // Has Region
                        (None, Some(_)) => 1, // Has Country only
                        (None, None) => 0,    // Has neither
                    }
                }

                let a_priority = priority(a);
                let b_priority = priority(b);
                b_priority.cmp(&a_priority)
            });

            // Apply the most specific rule for this custom tax
            if let Some(tax_rule) = applicable_rules.first() {
                let tax_amount = (rust_decimal::Decimal::from(item.amount) * tax_rule.rate)
                    .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                    .to_u64()
                    .unwrap_or(0);

                total_tax_amount += tax_amount;

                taxes.push(TaxItem {
                    tax_rate: tax_rule.rate,
                    tax_reference: custom_tax.reference.clone(),
                    tax_name: custom_tax.name.clone(),
                    tax_amount,
                });
            }
        }

        // If we found at least one applicable tax, return MultipleTaxes or Tax
        if !taxes.is_empty() {
            return if taxes.len() == 1 {
                let tax = taxes.into_iter().next().unwrap();
                TaxDetails::Tax {
                    tax_rate: tax.tax_rate,
                    tax_reference: tax.tax_reference,
                    tax_name: tax.tax_name,
                    tax_amount: tax.tax_amount,
                }
            } else {
                TaxDetails::MultipleTaxes {
                    taxes,
                    total_tax_amount,
                }
            };
        }
    }

    // Fall back to customer tax
    match customer_tax {
        CustomerTax::NoTax => TaxDetails::Exempt(VatExemptionReason::NotRegistered),
        CustomerTax::CustomTaxRate(rate) => {
            let tax_amount = (rust_decimal::Decimal::from(item.amount) * rate)
                .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                .to_u64()
                .unwrap_or(0);

            TaxDetails::Tax {
                tax_rate: *rate,
                tax_reference: "customer_custom".to_string(),
                tax_name: "Tax".to_string(),
                tax_amount,
            }
        }
        CustomerTax::CustomTaxRates(rates) => {
            use crate::model::TaxItem;

            let mut taxes = Vec::new();
            let mut total_tax_amount = 0u64;

            for rate in rates.iter() {
                let tax_amount = (rust_decimal::Decimal::from(item.amount) * rate.rate)
                    .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                    .to_u64()
                    .unwrap_or(0);

                total_tax_amount += tax_amount;

                taxes.push(TaxItem {
                    tax_rate: rate.rate,
                    tax_reference: rate.tax_code.clone(),
                    tax_name: rate.name.clone(),
                    tax_amount,
                });
            }

            TaxDetails::MultipleTaxes {
                taxes,
                total_tax_amount,
            }
        }
        CustomerTax::ResolvedTaxRate(rate) => {
            let name = get_tax_name(&rate.tax_type);
            let rate_decimal =
                rust_decimal::Decimal::from_f64(rate.rate).unwrap_or(rust_decimal::Decimal::ZERO);

            let tax_amount = (rust_decimal::Decimal::from(item.amount) * rate_decimal)
                .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                .to_u64()
                .unwrap_or(0);

            TaxDetails::Tax {
                tax_rate: rate_decimal,
                tax_reference: "customer_resolved".to_string(),
                tax_name: name.to_string(),
                tax_amount,
            }
        }
        CustomerTax::ResolvedMultipleTaxRates(rates) => {
            use crate::model::TaxItem;

            let mut taxes = Vec::new();
            let mut total_tax_amount = 0u64;

            for (idx, rate) in rates.iter().enumerate() {
                let name = get_tax_name(&rate.tax_type);
                let rate_decimal = rust_decimal::Decimal::from_f64(rate.rate)
                    .unwrap_or(rust_decimal::Decimal::ZERO);

                let tax_amount = (rust_decimal::Decimal::from(item.amount) * rate_decimal)
                    .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                    .to_u64()
                    .unwrap_or(0);

                total_tax_amount += tax_amount;

                taxes.push(TaxItem {
                    tax_rate: rate_decimal,
                    tax_reference: format!("customer_resolved_{}", idx),
                    tax_name: name.to_string(),
                    tax_amount,
                });
            }

            TaxDetails::MultipleTaxes {
                taxes,
                total_tax_amount,
            }
        }
        CustomerTax::Exempt => TaxDetails::Exempt(VatExemptionReason::TaxExempt),
    }
}

fn get_tax_name(tax_type: &world_tax::TaxType) -> &'static str {
    match tax_type {
        world_tax::TaxType::VAT(_) => "VAT",
        world_tax::TaxType::GST => "GST",
        world_tax::TaxType::HST => "HST",
        world_tax::TaxType::PST => "PST",
        world_tax::TaxType::QST => "QST",
        world_tax::TaxType::StateSalesTax => "Sales Tax",
    }
}

pub(crate) fn compute_breakdown_from_line_items(
    line_items: &[LineItemWithTax],
) -> CalculationResult {
    use ordermap::OrderMap;

    // Aggregate taxes by tax reference (OrderMap preserves insertion order)
    let mut tax_aggregates: OrderMap<String, (String, rust_decimal::Decimal, u64, u64)> =
        OrderMap::new();
    let mut exempt_items: Vec<(VatExemptionReason, u64)> = Vec::new();

    for item in line_items {
        match &item.tax_details {
            TaxDetails::Tax {
                tax_reference,
                tax_name,
                tax_rate,
                tax_amount,
            } => {
                tax_aggregates
                    .entry(tax_reference.clone())
                    .and_modify(|(_, _, taxable, tax)| {
                        *taxable += item.pre_tax_amount;
                        *tax += *tax_amount;
                    })
                    .or_insert((
                        tax_name.clone(),
                        *tax_rate,
                        item.pre_tax_amount,
                        *tax_amount,
                    ));
            }
            TaxDetails::MultipleTaxes { taxes, .. } => {
                // Each tax gets its own breakdown item
                for tax in taxes {
                    tax_aggregates
                        .entry(tax.tax_reference.clone())
                        .and_modify(|(_, _, taxable, tax_amt)| {
                            *taxable += item.pre_tax_amount;
                            *tax_amt += tax.tax_amount;
                        })
                        .or_insert((
                            tax.tax_name.clone(),
                            tax.tax_rate,
                            item.pre_tax_amount,
                            tax.tax_amount,
                        ));
                }
            }
            TaxDetails::Exempt(reason) => {
                exempt_items.push((reason.clone(), item.pre_tax_amount));
            }
        }
    }

    let mut breakdown: Vec<TaxBreakdownItem> = tax_aggregates
        .into_values()
        .map(
            |(tax_name, tax_rate, taxable_amount, tax_amount)| TaxBreakdownItem {
                taxable_amount,
                details: TaxDetails::Tax {
                    tax_rate,
                    tax_name,
                    tax_amount,
                    tax_reference: String::new(), // Not needed for breakdown
                },
            },
        )
        .collect();

    // Add exempt items grouped by reason
    let mut exempt_groups: OrderMap<VatExemptionReason, u64> = OrderMap::new();
    for (reason, amount) in exempt_items {
        *exempt_groups.entry(reason).or_default() += amount;
    }

    for (reason, taxable_amount) in exempt_groups {
        breakdown.push(TaxBreakdownItem {
            taxable_amount,
            details: TaxDetails::Exempt(reason),
        });
    }

    let total_tax: u64 = line_items
        .iter()
        .map(|item| match &item.tax_details {
            TaxDetails::Tax { tax_amount, .. } => *tax_amount,
            TaxDetails::MultipleTaxes {
                total_tax_amount, ..
            } => *total_tax_amount,
            TaxDetails::Exempt(_) => 0,
        })
        .sum();

    let total_amount_after_tax: u64 = line_items
        .iter()
        .map(|item| {
            item.pre_tax_amount
                + match &item.tax_details {
                    TaxDetails::Tax { tax_amount, .. } => *tax_amount,
                    TaxDetails::MultipleTaxes {
                        total_tax_amount, ..
                    } => *total_tax_amount,
                    TaxDetails::Exempt(_) => 0,
                }
        })
        .sum();
    CalculationResult {
        tax_amount: total_tax,
        total_amount_after_tax,
        breakdown,
        line_items: line_items.to_vec(),
    }
}
