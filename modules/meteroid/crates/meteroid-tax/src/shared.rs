use crate::TaxEngineError;
use crate::apply::{LineToTax, TaxToApply, apply_taxes};
use crate::model::{
    Address, CalculationResult, CustomerTax, LineItemForTax, LineItemWithTax, TaxBreakdownItem,
    TaxDetails, TaxItem, TaxKind, TaxRateRule, VatExemptionReason,
};
use error_stack::Report;
use ordermap::OrderMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

// Precedence ladder (finding C2). Tax for a line is resolved in exactly this
// order; the first rung that matches wins and stops the descent:
//   1. Exemption      — customer exempt, non-taxable product, or unregistered seller
//   2. Reverse charge — B2B cross-border shifts the liability to the buyer
//   3. Override        — a merchant-authored tax rate REPLACES the computed rate
//   4. Engine rate     — the resolved statutory (EU VAT) or manual rate
// Rungs 1-2 are customer-wide and short-circuit every line up front; rungs 3-4
// are decided per line in `determine_tax_details`. Replace-semantics are
// deliberate: an override at rung 3 fully supersedes the rung-4 engine rate.

/// One resolved tax on one line. Applying and rounding it is [`crate::apply`]'s job, so
/// nothing here is turned into money until every line has been resolved.
struct RawTax {
    tax_rate: Decimal,
    tax_reference: String,
    tax_name: String,
    /// What the levy is, for engine-resolved rates. `None` for merchant-authored
    /// custom taxes: their name is the merchant's own and no renderer may reword it.
    tax_kind: Option<TaxKind>,
    compound: bool,
}

/// Unrounded counterpart of [`TaxDetails`]. `Single` and `Multiple` decide the
/// rendered variant up front, so a one-entry `Multiple` stays `MultipleTaxes`.
enum RawTaxDetails {
    Exempt(VatExemptionReason),
    Single(RawTax),
    Multiple(Vec<RawTax>),
}

impl RawTaxDetails {
    fn taxes(&self) -> &[RawTax] {
        match self {
            RawTaxDetails::Exempt(_) => &[],
            RawTaxDetails::Single(tax) => std::slice::from_ref(tax),
            RawTaxDetails::Multiple(taxes) => taxes,
        }
    }

    fn into_details(self, amounts: &[i64]) -> TaxDetails {
        match self {
            RawTaxDetails::Exempt(reason) => TaxDetails::Exempt(reason),
            RawTaxDetails::Single(tax) => TaxDetails::Tax {
                tax_rate: tax.tax_rate,
                tax_reference: tax.tax_reference,
                tax_name: tax.tax_name,
                tax_kind: tax.tax_kind,
                tax_amount: amounts.first().copied().unwrap_or(0),
            },
            RawTaxDetails::Multiple(taxes) => {
                let taxes: Vec<TaxItem> = taxes
                    .into_iter()
                    .zip(amounts.iter().copied().chain(std::iter::repeat(0)))
                    .map(|(tax, tax_amount)| TaxItem {
                        tax_rate: tax.tax_rate,
                        tax_reference: tax.tax_reference,
                        tax_name: tax.tax_name,
                        tax_kind: tax.tax_kind,
                        tax_amount,
                    })
                    .collect();
                let total_tax_amount = taxes.iter().map(|tax| tax.tax_amount).sum();
                TaxDetails::MultipleTaxes {
                    taxes,
                    total_tax_amount,
                }
            }
        }
    }
}

/// A line whose taxes are resolved but not yet turned into money.
struct RawLineItem {
    line_id: String,
    amount: i64,
    tax_inclusive: bool,
    details: RawTaxDetails,
}

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
        CustomerTax::Export(basis) => Some(VatExemptionReason::Export(*basis)),
        CustomerTax::NoVatTerritory => Some(VatExemptionReason::NoVatTerritory),
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
                    effective_rate: Decimal::ZERO,
                    tax_details: TaxDetails::Exempt(reason),
                }
            })
            .collect();

        return Ok(computed_line_items);
    }

    let mut raw_line_items = Vec::with_capacity(line_items.len());
    for item in line_items {
        let tax_details = determine_tax_details(
            &item,
            &customer_tax,
            &invoicing_entity_address,
            &destination_address,
        );

        raw_line_items.push(RawLineItem {
            line_id: item.line_id,
            amount: item.amount,
            tax_inclusive: item.tax_inclusive,
            details: tax_details,
        });
    }

    Ok(round_and_allocate(raw_line_items))
}

/// Groups a tax across lines: coded rates by their accounting code, engine rates
/// (which carry none) by name AND rate, so 20% standard and 10% reduced stay apart.
fn tax_group_key(reference: &str, name: &str, rate: &Decimal) -> String {
    if reference.is_empty() {
        format!("name:{name}:{rate}")
    } else {
        format!("code:{reference}")
    }
}

/// Hands the resolved lines to the shared applier, which rounds each rate once over its
/// whole taxable base, then rebuilds the rendered details from the allocated amounts.
fn round_and_allocate(raw_line_items: Vec<RawLineItem>) -> Vec<LineItemWithTax> {
    let to_apply: Vec<LineToTax> = raw_line_items
        .iter()
        .map(|line| LineToTax {
            taxable_amount: line.amount,
            tax_inclusive: line.tax_inclusive,
            taxes: line
                .details
                .taxes()
                .iter()
                .map(|tax| TaxToApply {
                    rate: tax.tax_rate,
                    key: tax_group_key(&tax.tax_reference, &tax.tax_name, &tax.tax_rate),
                    compound: tax.compound,
                })
                .collect(),
        })
        .collect();

    raw_line_items
        .into_iter()
        .zip(apply_taxes(&to_apply))
        .zip(to_apply.iter())
        .map(|((line, amounts), applied)| LineItemWithTax {
            line_id: line.line_id,
            pre_tax_amount: applied.net_amount(&amounts),
            effective_rate: crate::apply::effective_rate(&applied.taxes),
            tax_details: line.details.into_details(&amounts),
        })
        .collect()
}

fn determine_tax_details(
    item: &LineItemForTax,
    customer_tax: &CustomerTax,
    invoicing_entity_address: &Address,
    destination_address: &Address,
) -> RawTaxDetails {
    // Rung 1 (line-level exemption): a non-taxable product yields no tax
    // regardless of jurisdiction, and it outranks any override or engine rate.
    // (Category-driven reduced/zero rates are a per-engine extension from here.)
    if item.tax_category.as_deref() == Some(crate::model::NONTAXABLE_CATEGORY_KEY) {
        return RawTaxDetails::Exempt(VatExemptionReason::TaxExempt);
    }

    // Rung 1 (line-level exemption): the seller must be registered somewhere for
    // any tax to apply.
    if invoicing_entity_address.country.is_none() {
        return RawTaxDetails::Exempt(VatExemptionReason::NotRegistered);
    }

    // Rung 3 (override): a merchant-authored tax rate REPLACES the engine rate.
    // Checked before the engine rate so replace-semantics hold.
    if let Some(override_details) = resolve_override(item, destination_address) {
        return override_details;
    }

    // Rung 4 (engine rate): the resolved statutory/manual rate.
    resolve_engine_rate(customer_tax)
}

/// Ladder rung 3: apply the most-specific destination-matched override rule per
/// custom tax on the line, if any matches. Returns `None` when no rule matches,
/// letting the caller descend to the engine rate (rung 4).
fn resolve_override(item: &LineItemForTax, destination_address: &Address) -> Option<RawTaxDetails> {
    if item.custom_taxes.is_empty() {
        return None;
    }

    // Override rules are destination-scoped: they match the customer's
    // ship-to (falling back to billing), not the seller's own address.
    let destination_country = destination_address.country.as_ref();

    let mut taxes = Vec::new();

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
            taxes.push(RawTax {
                tax_rate: tax_rule.rate,
                tax_reference: custom_tax.reference.clone(),
                tax_name: custom_tax.name.clone(),
                tax_kind: None,
                compound: false,
            });
        }
    }

    // If we found at least one applicable tax, return MultipleTaxes or Tax
    if taxes.is_empty() {
        return None;
    }

    Some(if taxes.len() == 1 {
        RawTaxDetails::Single(taxes.into_iter().next().unwrap())
    } else {
        RawTaxDetails::Multiple(taxes)
    })
}

/// Ladder rung 4: the resolved statutory (EU VAT) or manual engine rate.
fn resolve_engine_rate(customer_tax: &CustomerTax) -> RawTaxDetails {
    match customer_tax {
        CustomerTax::NoTax => RawTaxDetails::Exempt(VatExemptionReason::NotRegistered),
        CustomerTax::TaxRates(rates) => RawTaxDetails::Multiple(
            rates
                .iter()
                .map(|rate| RawTax {
                    tax_rate: rate.rate,
                    tax_reference: rate.tax_code.clone(),
                    tax_name: rate.name.clone(),
                    tax_kind: None,
                    compound: false,
                })
                .collect(),
        ),
        CustomerTax::ResolvedTaxRate(rate) => match &rate.tax_type {
            // The resolved scenario can be exempt or reverse charge (export zero,
            // intra-EU B2B): render those as exemptions, not a 0% tax line.
            world_tax::TaxType::VAT(world_tax::VatRate::Exempt) => {
                RawTaxDetails::Exempt(VatExemptionReason::TaxExempt)
            }
            world_tax::TaxType::VAT(world_tax::VatRate::ReverseCharge) => {
                RawTaxDetails::Exempt(VatExemptionReason::ReverseCharge)
            }
            _ => {
                let rate_decimal = Decimal::from_f64(rate.rate).unwrap_or(Decimal::ZERO);

                RawTaxDetails::Single(RawTax {
                    tax_rate: rate_decimal,
                    tax_reference: String::new(),
                    tax_name: get_tax_name(&rate.tax_type).to_string(),
                    tax_kind: Some(tax_kind_of(&rate.tax_type)),
                    compound: false,
                })
            }
        },
        CustomerTax::ResolvedMultipleTaxRates(rates) => RawTaxDetails::Multiple(
            // Compound rates (e.g. Canadian QST on top of GST) levy on the base plus the tax
            // accrued so far; the applier builds that chain when it computes the amounts.
            rates
                .iter()
                .map(|rate| RawTax {
                    tax_rate: Decimal::from_f64(rate.rate).unwrap_or(Decimal::ZERO),
                    tax_reference: String::new(),
                    tax_name: get_tax_name(&rate.tax_type).to_string(),
                    tax_kind: Some(tax_kind_of(&rate.tax_type)),
                    compound: rate.compound,
                })
                .collect(),
        ),
        CustomerTax::Exempt => RawTaxDetails::Exempt(VatExemptionReason::TaxExempt),
        CustomerTax::Export(basis) => RawTaxDetails::Exempt(VatExemptionReason::Export(*basis)),
        CustomerTax::NoVatTerritory => RawTaxDetails::Exempt(VatExemptionReason::NoVatTerritory),
        CustomerTax::ReverseCharge => RawTaxDetails::Exempt(VatExemptionReason::ReverseCharge),
    }
}

// The engine's own English label for a resolved rate. It stays alongside the kind:
// it is the grouping key, what the API and webhooks expose, and the persisted
// fallback for breakdown rows written before the kind existed. Document renderers
// word the *kind* in the presentation language instead of printing this.
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

/// The provider type as a fact about the levy, recorded so renderers can word the
/// row label in the document's language rather than parroting `get_tax_name`'s
/// English (a German invoice labelling its rate "VAT" instead of "USt.").
fn tax_kind_of(tax_type: &world_tax::TaxType) -> TaxKind {
    match tax_type {
        world_tax::TaxType::VAT(_) => TaxKind::Vat,
        world_tax::TaxType::GST => TaxKind::Gst,
        world_tax::TaxType::HST => TaxKind::Hst,
        world_tax::TaxType::PST => TaxKind::Pst,
        world_tax::TaxType::QST => TaxKind::Qst,
        world_tax::TaxType::StateSalesTax => TaxKind::SalesTax,
    }
}

pub(crate) fn compute_breakdown_from_line_items(
    line_items: &[LineItemWithTax],
) -> CalculationResult {
    // Aggregate taxes into breakdown lines. Real tax rates carry an accounting
    // code (W1) which becomes the breakdown key and reference; engine-computed
    // VAT has no code, so those lines aggregate by name and expose no reference.
    // Value: (tax_reference, tax_name, tax_kind, tax_rate, taxable_amount, tax_amount).
    // The kind rides on the group's first entry: the name is derived from the kind,
    // so two entries sharing a group key by name share the kind by construction.
    #[allow(clippy::type_complexity)]
    let mut tax_aggregates: OrderMap<
        String,
        (String, String, Option<TaxKind>, Decimal, i64, i64),
    > = OrderMap::new();
    let mut exempt_items: Vec<(VatExemptionReason, i64)> = Vec::new();

    for item in line_items {
        match &item.tax_details {
            TaxDetails::Tax {
                tax_reference,
                tax_name,
                tax_kind,
                tax_rate,
                tax_amount,
            } => {
                tax_aggregates
                    .entry(tax_group_key(tax_reference, tax_name, tax_rate))
                    .and_modify(|(_, _, _, _, taxable, tax)| {
                        *taxable += item.pre_tax_amount;
                        *tax += *tax_amount;
                    })
                    .or_insert((
                        tax_reference.clone(),
                        tax_name.clone(),
                        *tax_kind,
                        *tax_rate,
                        item.pre_tax_amount,
                        *tax_amount,
                    ));
            }
            TaxDetails::MultipleTaxes { taxes, .. } => {
                // Each tax gets its own breakdown item
                for tax in taxes {
                    tax_aggregates
                        .entry(tax_group_key(
                            &tax.tax_reference,
                            &tax.tax_name,
                            &tax.tax_rate,
                        ))
                        .and_modify(|(_, _, _, _, taxable, tax_amt)| {
                            *taxable += item.pre_tax_amount;
                            *tax_amt += tax.tax_amount;
                        })
                        .or_insert((
                            tax.tax_reference.clone(),
                            tax.tax_name.clone(),
                            tax.tax_kind,
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
            |(tax_reference, tax_name, tax_kind, tax_rate, taxable_amount, tax_amount)| {
                TaxBreakdownItem {
                    taxable_amount,
                    details: TaxDetails::Tax {
                        tax_rate,
                        tax_name,
                        tax_kind,
                        tax_amount,
                        tax_reference,
                    },
                }
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
