use crate::TaxEngineError;
use crate::model::*;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

pub(crate) async fn compute_tax(
    customer_tax: CustomerTax,
    invoicing_entity_address: Address,
    line_items: Vec<LineItemForTax>,
) -> error_stack::Result<Vec<LineItemWithTax>, TaxEngineError> {
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
    let invoicing_entity_country = match &invoicing_entity_address.country {
        Some(country) => country,
        None => return TaxDetails::Exempt(VatExemptionReason::NotRegistered),
    };

    // First check custom tax on line item
    if let Some(custom_tax) = &item.custom_tax {
        let mut tax_rule: Vec<&TaxRule> = custom_tax
            .tax_rules
            .iter()
            .filter(|c| {
                let mut include = true;

                if let Some(country) = &c.country {
                    include = country == invoicing_entity_country
                }

                if let Some(region) = &c.region {
                    if let Some(ie_region) = &invoicing_entity_address.region {
                        include = include && region == ie_region;
                    } else {
                        include = false;
                    }
                }

                include
            })
            .collect();

        tax_rule.sort_by(|a, b| {
            fn priority(a: &TaxRule) -> i32 {
                match (&a.region, &a.country) {
                    (Some(_), _) => 2,    // Has Region
                    (None, Some(_)) => 1, // Has Country only
                    (None, None) => 0,    // Has neither
                }
            }

            let a_priority = priority(a);
            let b_priority = priority(b);
            b_priority.cmp(&a_priority)
        });

        if let Some(tax_rule) = tax_rule.first() {
            let tax_amount = (rust_decimal::Decimal::from(item.amount) * tax_rule.rate)
                .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                .to_u64()
                .unwrap_or(0);

            return TaxDetails::Tax {
                tax_rate: tax_rule.rate,
                tax_reference: custom_tax.reference.clone(),
                tax_name: custom_tax.name.clone(),
                tax_amount,
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
        CustomerTax::ResolvedTaxRate(rate) => {
            let name = match rate.tax_type {
                world_tax::TaxType::VAT(_) => "VAT",
                world_tax::TaxType::GST => "GST",
                world_tax::TaxType::HST => "HST",
                world_tax::TaxType::PST => "PST",
                world_tax::TaxType::QST => "QST",
                world_tax::TaxType::StateSalesTax => "Sales Tax",
            };

            let rate =
                rust_decimal::Decimal::from_f64(rate.rate).unwrap_or(rust_decimal::Decimal::ZERO);

            let tax_amount = (rust_decimal::Decimal::from(item.amount) * rate)
                .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                .to_u64()
                .unwrap_or(0);

            TaxDetails::Tax {
                tax_rate: rate,
                tax_reference: "customer_resolved".to_string(),
                tax_name: name.to_string(),
                tax_amount,
            }
        }
        CustomerTax::Exempt => TaxDetails::Exempt(VatExemptionReason::TaxExempt),
    }
}

pub(crate) fn compute_breakdown_from_line_items(
    line_items: &[LineItemWithTax],
) -> CalculationResult {
    use std::collections::HashMap;

    // Group by tax reference or exemption type
    let mut groups: HashMap<String, Vec<&LineItemWithTax>> = HashMap::new();

    for item in line_items {
        let key = match &item.tax_details {
            TaxDetails::Tax { tax_reference, .. } => tax_reference.clone(),
            TaxDetails::Exempt(reason) => format!("exempt_{:?}", reason),
        };
        groups.entry(key).or_default().push(item);
    }

    let breakdown = groups
        .into_values()
        .map(|items| {
            let total_taxable_amount: u64 = items.iter().map(|item| item.pre_tax_amount).sum();

            match &items[0].tax_details {
                TaxDetails::Exempt(reason) => TaxBreakdownItem {
                    taxable_amount: total_taxable_amount,
                    details: TaxDetails::Exempt(reason.clone()),
                },
                TaxDetails::Tax {
                    tax_rate, tax_name, ..
                } => {
                    let total_tax_amount: u64 = items
                        .iter()
                        .map(|item| match &item.tax_details {
                            TaxDetails::Tax { tax_amount, .. } => *tax_amount,
                            _ => 0,
                        })
                        .sum();

                    TaxBreakdownItem {
                        taxable_amount: total_taxable_amount,
                        details: TaxDetails::Tax {
                            tax_rate: *tax_rate,
                            tax_name: tax_name.clone(),
                            tax_amount: total_tax_amount,
                            tax_reference: String::new(), // Not needed for breakdown
                        },
                    }
                }
            }
        })
        .collect();

    let total_tax: u64 = line_items
        .iter()
        .map(|item| match &item.tax_details {
            TaxDetails::Tax { tax_amount, .. } => *tax_amount,
            TaxDetails::Exempt(_) => 0,
        })
        .sum();

    let total_amount_after_tax: u64 = line_items
        .iter()
        .map(|item| {
            item.pre_tax_amount
                + match &item.tax_details {
                    TaxDetails::Tax { tax_amount, .. } => *tax_amount,
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
