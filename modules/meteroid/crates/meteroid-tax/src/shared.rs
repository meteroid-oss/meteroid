use crate::TaxEngineError;
use crate::model::{
    Address, CalculationResult, CustomerTax, LineItemForTax, LineItemWithTax, TaxBreakdownItem,
    TaxDetails, TaxItem, TaxRateRule, VatExemptionReason,
};
use error_stack::Report;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

// Precedence ladder (finding C2). Tax for a line is resolved in exactly this
// order; the first rung that matches wins and stops the descent:
//   1. Exemption      — customer exempt, non-taxable product, or unregistered seller
//   2. Reverse charge — B2B cross-border shifts the liability to the buyer
//   3. Override        — a merchant-authored tax rate REPLACES the computed rate
//   4. Engine rate     — the resolved statutory (EU VAT) or manual rate
// Rungs 1-2 are customer-wide and short-circuit every line up front; rungs 3-4
// are decided per line in `determine_tax_details`. Replace-semantics are
// deliberate: an override at rung 3 fully supersedes the rung-4 engine rate.

pub(crate) async fn compute_tax(
    customer_tax: CustomerTax,
    invoicing_entity_address: Address,
    destination_address: Address,
    line_items: Vec<LineItemForTax>,
) -> Result<Vec<LineItemWithTax>, Report<TaxEngineError>> {
    // Ladder rungs 1-2: customer-wide exemption / reverse charge. These outrank
    // any line-level override, so they are resolved before line processing.
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
        CustomerTax::ReverseCharge => Some(VatExemptionReason::ReverseCharge),
        _ => None,
    };

    if let Some(exemption) = customer_exemption {
        // Customer-wide exemption / reverse charge: no tax on any line. Rung 1
        // (exemption) still outranks rung 2 (reverse charge): a non-taxable
        // product stays TaxExempt even for a reverse-charge customer.
        let computed_line_items: Vec<LineItemWithTax> = line_items
            .into_iter()
            .map(|item| {
                let reason = if item.tax_category.as_deref()
                    == Some(crate::model::NONTAXABLE_CATEGORY_KEY)
                {
                    VatExemptionReason::TaxExempt
                } else {
                    exemption.clone()
                };
                LineItemWithTax {
                    line_id: item.line_id,
                    pre_tax_amount: item.amount,
                    tax_details: TaxDetails::Exempt(reason),
                }
            })
            .collect();

        return Ok(computed_line_items);
    }

    let mut computed_line_items = Vec::new();
    for item in line_items {
        let tax_details = determine_tax_details(
            &item,
            &customer_tax,
            &invoicing_entity_address,
            &destination_address,
        );

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
    destination_address: &Address,
) -> TaxDetails {
    // Rung 1 (line-level exemption): a non-taxable product yields no tax
    // regardless of jurisdiction, and it outranks any override or engine rate.
    // (Category-driven reduced/zero rates are a per-engine extension from here.)
    if item.tax_category.as_deref() == Some(crate::model::NONTAXABLE_CATEGORY_KEY) {
        return TaxDetails::Exempt(VatExemptionReason::TaxExempt);
    }

    // Rung 1 (line-level exemption): the seller must be registered somewhere for
    // any tax to apply.
    if invoicing_entity_address.country.is_none() {
        return TaxDetails::Exempt(VatExemptionReason::NotRegistered);
    }

    // Rung 3 (override): a merchant-authored tax rate REPLACES the engine rate.
    // Checked before the engine rate so replace-semantics hold.
    if let Some(override_details) = resolve_override(item, destination_address) {
        return override_details;
    }

    // Rung 4 (engine rate): the resolved statutory/manual rate.
    resolve_engine_rate(item, customer_tax)
}

/// Ladder rung 3: apply the most-specific destination-matched override rule per
/// custom tax on the line, if any matches. Returns `None` when no rule matches,
/// letting the caller descend to the engine rate (rung 4).
fn resolve_override(item: &LineItemForTax, destination_address: &Address) -> Option<TaxDetails> {
    if item.custom_taxes.is_empty() {
        return None;
    }

    // Override rules are destination-scoped: they match the customer's
    // ship-to (falling back to billing), not the seller's own address.
    let destination_country = destination_address.country.as_ref();

    let mut taxes = Vec::new();
    let mut total_tax_amount = 0i64;

    for custom_tax in &item.custom_taxes {
        // Find the most specific applicable rule for this tax
        let mut applicable_rules: Vec<&TaxRateRule> = custom_tax
            .tax_rules
            .iter()
            .filter(|rule| {
                let mut include = true;

                // A rule with no country is an "everywhere" fallback.
                if let Some(country) = &rule.country {
                    include = Some(country) == destination_country;
                }

                if let Some(region) = &rule.region {
                    if let Some(dest_region) = &destination_address.region {
                        include = include && region == dest_region;
                    } else {
                        include = false;
                    }
                }

                include
            })
            .collect();

        // Sort by specificity (region > country > none)
        applicable_rules.sort_by(|a, b| {
            fn priority(rule: &TaxRateRule) -> i32 {
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
                .to_i64()
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
    if taxes.is_empty() {
        return None;
    }

    Some(if taxes.len() == 1 {
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
    })
}

/// Ladder rung 4: the resolved statutory (EU VAT) or manual engine rate.
fn resolve_engine_rate(item: &LineItemForTax, customer_tax: &CustomerTax) -> TaxDetails {
    match customer_tax {
        CustomerTax::NoTax => TaxDetails::Exempt(VatExemptionReason::NotRegistered),
        CustomerTax::TaxRates(rates) => {
            let mut taxes = Vec::new();
            let mut total_tax_amount = 0i64;

            for rate in rates.iter() {
                let tax_amount = (rust_decimal::Decimal::from(item.amount) * rate.rate)
                    .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                    .to_i64()
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
        CustomerTax::ResolvedTaxRate(rate) => match &rate.tax_type {
            // The resolved scenario can be exempt or reverse charge (export zero,
            // intra-EU B2B): render those as exemptions, not a 0% tax line.
            world_tax::TaxType::VAT(world_tax::VatRate::Exempt) => {
                TaxDetails::Exempt(VatExemptionReason::TaxExempt)
            }
            world_tax::TaxType::VAT(world_tax::VatRate::ReverseCharge) => {
                TaxDetails::Exempt(VatExemptionReason::ReverseCharge)
            }
            _ => {
                let name = get_tax_name(&rate.tax_type);
                let rate_decimal = rust_decimal::Decimal::from_f64(rate.rate)
                    .unwrap_or(rust_decimal::Decimal::ZERO);

                let tax_amount = (rust_decimal::Decimal::from(item.amount) * rate_decimal)
                    .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                    .to_i64()
                    .unwrap_or(0);

                TaxDetails::Tax {
                    tax_rate: rate_decimal,
                    tax_reference: String::new(),
                    tax_name: name.to_string(),
                    tax_amount,
                }
            }
        },
        CustomerTax::ResolvedMultipleTaxRates(rates) => {
            let mut taxes = Vec::new();
            let mut total_tax_amount = 0i64;

            // Compound rates (e.g. Canadian QST on top of GST) levy on the base
            // plus the tax accrued so far; non-compound rates levy on the base
            // alone. The EU dataset carries no compound rows, so this is a no-op
            // there, but keeps a stacked-tax jurisdiction correct.
            let base = rust_decimal::Decimal::from(item.amount);
            let mut running_tax = rust_decimal::Decimal::ZERO;

            for rate in rates.iter() {
                let name = get_tax_name(&rate.tax_type);
                let rate_decimal = rust_decimal::Decimal::from_f64(rate.rate)
                    .unwrap_or(rust_decimal::Decimal::ZERO);

                let taxable = if rate.compound {
                    base + running_tax
                } else {
                    base
                };

                let tax_decimal = taxable * rate_decimal;
                running_tax += tax_decimal;

                let tax_amount = tax_decimal
                    .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                    .to_i64()
                    .unwrap_or(0);

                total_tax_amount += tax_amount;

                taxes.push(TaxItem {
                    tax_rate: rate_decimal,
                    tax_reference: String::new(),
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
        CustomerTax::ReverseCharge => TaxDetails::Exempt(VatExemptionReason::ReverseCharge),
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

    // Aggregate taxes into breakdown lines. Real tax rates carry an accounting
    // code (W1) which becomes the breakdown key and reference; engine-computed
    // VAT has no code, so those lines aggregate by name and expose no reference.
    // Value: (tax_reference, tax_name, tax_rate, taxable_amount, tax_amount).
    let mut tax_aggregates: OrderMap<String, (String, String, rust_decimal::Decimal, i64, i64)> =
        OrderMap::new();
    let mut exempt_items: Vec<(VatExemptionReason, i64)> = Vec::new();

    // Real tax rates group by their accounting code; engine-computed rates carry
    // no code, so they group by name AND rate — this keeps distinct classes
    // (e.g. 20% standard vs 10% reduced VAT) as separate breakdown lines.
    let aggregate_key = |reference: &str, name: &str, rate: &rust_decimal::Decimal| {
        if reference.is_empty() {
            format!("name:{name}:{rate}")
        } else {
            format!("code:{reference}")
        }
    };

    for item in line_items {
        match &item.tax_details {
            TaxDetails::Tax {
                tax_reference,
                tax_name,
                tax_rate,
                tax_amount,
            } => {
                tax_aggregates
                    .entry(aggregate_key(tax_reference, tax_name, tax_rate))
                    .and_modify(|(_, _, _, taxable, tax)| {
                        *taxable += item.pre_tax_amount;
                        *tax += *tax_amount;
                    })
                    .or_insert((
                        tax_reference.clone(),
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
                        .entry(aggregate_key(
                            &tax.tax_reference,
                            &tax.tax_name,
                            &tax.tax_rate,
                        ))
                        .and_modify(|(_, _, _, taxable, tax_amt)| {
                            *taxable += item.pre_tax_amount;
                            *tax_amt += tax.tax_amount;
                        })
                        .or_insert((
                            tax.tax_reference.clone(),
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
            |(tax_reference, tax_name, tax_rate, taxable_amount, tax_amount)| TaxBreakdownItem {
                taxable_amount,
                details: TaxDetails::Tax {
                    tax_rate,
                    tax_name,
                    tax_amount,
                    tax_reference,
                },
            },
        )
        .collect();

    // Add exempt items grouped by reason
    let mut exempt_groups: OrderMap<VatExemptionReason, i64> = OrderMap::new();
    for (reason, amount) in exempt_items {
        *exempt_groups.entry(reason).or_default() += amount;
    }

    for (reason, taxable_amount) in exempt_groups {
        breakdown.push(TaxBreakdownItem {
            taxable_amount,
            details: TaxDetails::Exempt(reason),
        });
    }

    let total_tax: i64 = line_items
        .iter()
        .map(|item| match &item.tax_details {
            TaxDetails::Tax { tax_amount, .. } => *tax_amount,
            TaxDetails::MultipleTaxes {
                total_tax_amount, ..
            } => *total_tax_amount,
            TaxDetails::Exempt(_) => 0,
        })
        .sum();

    let total_amount_after_tax: i64 = line_items
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
        exemption_reason: None,
    }
}
