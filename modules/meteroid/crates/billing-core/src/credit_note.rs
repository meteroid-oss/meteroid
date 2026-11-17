//! What a credit note takes back of the lines it credits: each amount its cumulative share of
//! the line (R-005), the tax details shared to the cent (R-074).

use common_utils::decimals::ToUnit;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

/// A credited amount beyond what minor units in `i64` hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overflow;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxDetail {
    pub tax_rate: Decimal,
    pub tax_name: String,
    pub tax_amount: i64,
}

/// What crediting `credited` more of a line billed `original` takes of its `amount`, after
/// `already` was credited: each cumulative share rounded half-up (OQ14), so notes completing the
/// line take back exactly its amount, a half cent never twice.
pub fn cumulative_share(amount: i64, already: i64, credited: i64, original: i64) -> i64 {
    if original == 0 {
        return 0;
    }
    let share = |credited: i64| {
        (Decimal::from(amount) * Decimal::from(credited) / Decimal::from(original))
            .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
            .unwrap_or(0)
    };
    share(already.saturating_add(credited)) - share(already)
}

/// `cumulative_share` less what the line's earlier notes actually `returned` of `amount` rather than
/// what this rule would have: a note issued under another rounding rule is completed exactly.
/// `None` trusts the earlier notes to have followed this rule.
pub fn share_net_of_returned(
    amount: i64,
    returned: Option<i64>,
    (already, credited, original): (i64, i64, i64),
) -> i64 {
    let Some(returned) = returned else {
        return cumulative_share(amount, already, credited, original);
    };
    let share = cumulative_share(amount, 0, already.saturating_add(credited), original) - returned;
    // Earlier notes that returned more leave nothing for this one, never a negative share.
    if (share >= 0) == (amount >= 0) {
        share
    } else {
        0
    }
}

/// What a line's earlier non-voided notes returned of its taxable amount and tax, positive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Returned {
    taxable: i64,
    tax: i64,
    details: Vec<TaxDetail>,
}

impl Returned {
    /// Adds an earlier note's line, whose amounts are negative.
    pub fn add(
        &mut self,
        taxable_amount: i64,
        tax_amount: i64,
        tax_details: &[TaxDetail],
    ) -> Result<(), Overflow> {
        self.taxable = self.taxable.checked_sub(taxable_amount).ok_or(Overflow)?;
        self.tax = self.tax.checked_sub(tax_amount).ok_or(Overflow)?;
        for detail in tax_details {
            match self.details.iter_mut().find(|returned| {
                returned.tax_name == detail.tax_name && returned.tax_rate == detail.tax_rate
            }) {
                Some(returned) => {
                    returned.tax_amount = returned
                        .tax_amount
                        .checked_sub(detail.tax_amount)
                        .ok_or(Overflow)?;
                }
                None => self.details.push(TaxDetail {
                    tax_amount: detail.tax_amount.checked_neg().ok_or(Overflow)?,
                    ..detail.clone()
                }),
            }
        }
        Ok(())
    }

    pub fn taxable(&self) -> i64 {
        self.taxable
    }

    pub fn tax(&self) -> i64 {
        self.tax
    }

    fn detail(&self, detail: &TaxDetail) -> i64 {
        self.details
            .iter()
            .filter(|returned| {
                returned.tax_name == detail.tax_name && returned.tax_rate == detail.tax_rate
            })
            .map(|returned| returned.tax_amount)
            .sum()
    }
}

/// Prorates each tax detail like the line's tax (`share_net_of_returned`), negates it, and spreads
/// the residual to the cent of `expected_sum` from the largest detail down, each within what its
/// earlier notes left of it: a detail never returns more than it bore.
pub fn prorate_and_negate_tax_details(
    details: &[TaxDetail],
    shares: (i64, i64, i64),
    returned: Option<&Returned>,
    expected_sum: i64,
) -> Vec<TaxDetail> {
    if details.is_empty() {
        return Vec::new();
    }
    let (already, _, original) = shares;
    // Details of one line share its sign; work in returned magnitudes.
    let sign = if details.iter().map(|d| d.tax_amount).sum::<i64>() < 0 {
        -1
    } else {
        1
    };
    let mut taken: Vec<i64> = Vec::with_capacity(details.len());
    let mut left: Vec<i64> = Vec::with_capacity(details.len());
    for d in details {
        let returned_before = returned
            .map(|returned| returned.detail(d))
            .unwrap_or_else(|| cumulative_share(d.tax_amount, 0, already, original));
        taken.push(
            sign * share_net_of_returned(
                d.tax_amount,
                returned.map(|returned| returned.detail(d)),
                shares,
            ),
        );
        left.push((sign * (d.tax_amount - returned_before)).max(0));
    }

    let mut order: Vec<usize> = (0..details.len()).collect();
    order.sort_by_key(|i| std::cmp::Reverse(details[*i].tax_amount.unsigned_abs()));
    let mut residual = -sign * expected_sum - taken.iter().sum::<i64>();
    for i in order {
        if residual == 0 {
            break;
        }
        let room = if residual > 0 {
            (left[i] - taken[i]).max(0)
        } else {
            taken[i].max(0)
        };
        let moved = residual.signum() * residual.abs().min(room);
        taken[i] += moved;
        residual -= moved;
    }
    // Unplaceable only if the line's own figures disagree: the largest detail keeps the cent.
    if residual != 0
        && let Some(i) = (0..details.len()).max_by_key(|i| details[*i].tax_amount.unsigned_abs())
    {
        taken[i] += residual;
    }

    details
        .iter()
        .zip(taken)
        .map(|(d, taken)| TaxDetail {
            tax_rate: d.tax_rate,
            tax_name: d.tax_name.clone(),
            tax_amount: -sign * taken,
        })
        .collect()
}

/// Funded credit a partial credit of a `Credit` line reverses, rounded cumulatively over the
/// line's notes so that crediting the whole subtotal always reverses the whole grant.
pub fn credit_grant_reversal(
    face: i64,
    already_credited: i64,
    credit_subtotal: i64,
    original_subtotal: i64,
) -> Option<i64> {
    if already_credited < 0 {
        return None;
    }
    let after = already_credited.checked_add(credit_subtotal)?;
    cumulative_grant_reversal(face, after, original_subtotal)?.checked_sub(
        cumulative_grant_reversal(face, already_credited, original_subtotal)?,
    )
}

/// Credit reversed once `credited` of the line's subtotal is credited: pro rata, half up, never
/// above the face.
pub fn cumulative_grant_reversal(face: i64, credited: i64, original_subtotal: i64) -> Option<i64> {
    if original_subtotal <= 0 || face < 0 || credited < 0 {
        return None;
    }
    let credited = credited.min(original_subtotal);
    let numerator = i128::from(face).checked_mul(i128::from(credited))?;
    let denominator = i128::from(original_subtotal);
    let (quotient, remainder) = (numerator / denominator, numerator % denominator);
    let prorated = if remainder * 2 >= denominator {
        quotient + 1
    } else {
        quotient
    };
    let prorated = i64::try_from(prorated).ok()?;
    Some(prorated.min(face))
}

/// Quantity and unit price that reconcile to `credited` (minor units): the line's real unit
/// count and the effective rate, never a fabricated fractional quantity.
pub fn effective_rate_display(
    quantity: Option<Decimal>,
    credited: i64,
    precision: u8,
) -> (Option<Decimal>, Option<Decimal>) {
    let qty = quantity
        .filter(|q| *q > Decimal::ZERO)
        .unwrap_or(Decimal::ONE);
    (Some(qty), Some(credited.to_unit(precision) / qty))
}

/// The amounts of an invoice line a note credits.
#[derive(Clone, Debug, PartialEq)]
pub struct BilledLine {
    pub amount_subtotal: i64,
    pub taxable_amount: i64,
    pub tax_amount: i64,
    pub tax_details: Vec<TaxDetail>,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub is_prorated: bool,
    pub credit_grant_amount: Option<i64>,
    pub promotional_credit: Option<i64>,
}

/// How much of a line a note credits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Credit<'a> {
    /// Subtotal (excl. tax) to credit, in minor units. `None` credits the whole line as billed.
    pub subtotal: Option<i64>,
    /// Subtotal earlier non-voided notes already credited on the line.
    pub already_credited: i64,
    /// What those notes returned; `None` assumes they followed the cumulative rule.
    pub returned: Option<&'a Returned>,
    /// The note lists sub-lines of its own: the line states no quantity or price.
    pub sub_lines_overridden: bool,
}

/// The note's line, negative.
#[derive(Clone, Debug, PartialEq)]
pub struct CreditedLine {
    pub amount_subtotal: i64,
    pub taxable_amount: i64,
    pub tax_amount: i64,
    pub amount_total: i64,
    pub tax_details: Vec<TaxDetail>,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub is_prorated: bool,
    pub credit_grant_amount: Option<i64>,
    pub promotional_credit: Option<i64>,
    /// Whether the note line keeps the credit grant the line sold (its expiry, unit and tax).
    pub keeps_grant: bool,
}

/// The note line crediting `credit` of `item` (R-005). `from_proration`: the credit is a
/// time-proration of what was billed (an amendment), so a smaller credit is a prorated line.
pub fn negate_line(
    item: &BilledLine,
    credit: Credit<'_>,
    precision: u8,
    from_proration: bool,
) -> CreditedLine {
    let Some(credit_subtotal) = credit.subtotal else {
        // Full credit: negate the amounts, keep original quantity and positive
        // unit_price (the sign lives in the amounts).
        // Note: amount_total is recalculated as taxable_amount + tax_amount to properly
        // account for discounts (which reduce taxable_amount but not amount_subtotal)
        // A quantity without a price (usage priced in a credit unit, whose money is only
        // the overage) is stated at its effective rate, or qty × price ≠ amount (R120).
        let (quantity, unit_price) = match (item.quantity, item.unit_price) {
            (Some(_), None) => {
                effective_rate_display(item.quantity, item.amount_subtotal, precision)
            }
            shown => shown,
        };
        return CreditedLine {
            amount_subtotal: -item.amount_subtotal,
            taxable_amount: -item.taxable_amount,
            tax_amount: -item.tax_amount,
            amount_total: -(item.taxable_amount + item.tax_amount),
            tax_details: item
                .tax_details
                .iter()
                .map(|d| TaxDetail {
                    tax_rate: d.tax_rate,
                    tax_name: d.tax_name.clone(),
                    tax_amount: -d.tax_amount,
                })
                .collect(),
            quantity,
            unit_price,
            is_prorated: item.is_prorated,
            credit_grant_amount: item.credit_grant_amount.and_then(i64::checked_neg),
            promotional_credit: item.promotional_credit.and_then(i64::checked_neg),
            keeps_grant: true,
        };
    };

    // Partial credit: amount is the subtotal (excl. tax)
    let original_subtotal = item.amount_subtotal;
    if original_subtotal == 0 {
        // Avoid division by zero - return zero amounts
        return CreditedLine {
            amount_subtotal: 0,
            taxable_amount: 0,
            tax_amount: 0,
            amount_total: 0,
            tax_details: item.tax_details.clone(),
            quantity: item.quantity,
            unit_price: Some(Decimal::ZERO),
            is_prorated: item.is_prorated,
            credit_grant_amount: None,
            promotional_credit: None,
            keeps_grant: false,
        };
    }

    let credited_grant_amount = item.credit_grant_amount.and_then(|face| {
        credit_grant_reversal(
            face,
            credit.already_credited,
            credit_subtotal,
            original_subtotal,
        )?
        .checked_neg()
    });

    // Taxable amount and tax are prorated by the credited share, cumulatively over
    // the line's earlier notes, half-up.
    let shares = (
        credit.already_credited.max(0),
        credit_subtotal,
        original_subtotal,
    );
    let returned = credit.returned;
    let prorated_taxable = share_net_of_returned(
        item.taxable_amount,
        returned.map(|returned| returned.taxable),
        shares,
    );
    let prorated_tax = share_net_of_returned(
        item.tax_amount,
        returned.map(|returned| returned.tax),
        shares,
    );

    // So is the promotional credit the line took before tax, which goes back to its
    // grants: cumulatively, so the note completing the line returns all of it.
    let prorated_promotional = item.promotional_credit.and_then(|share| {
        credit_grant_reversal(
            share,
            credit.already_credited,
            credit_subtotal,
            original_subtotal,
        )
        .filter(|share| *share != 0)
        .map(|share| -share)
    });

    // Prorate each tax detail so the breakdown reflects the credited portion,
    // then negate and apply a residual correction so the sum matches -prorated_tax
    // to the cent (largest-magnitude detail absorbs the rounding delta).
    let prorated_tax_details =
        prorate_and_negate_tax_details(&item.tax_details, shares, returned, -prorated_tax);

    // Total = taxable + tax (uses taxable to account for discounts)
    let credit_total = prorated_taxable + prorated_tax;

    // Effective-rate display so qty × unit_price always reconciles to the credited
    // amount: quantity = the line's real unit count, unit_price = amount ÷ quantity
    // (the effective, possibly-prorated, per-unit rate). Never divide to fabricate a
    // fractional quantity (that produced the repeating decimal). Sub-line overrides
    // keep the parent blank — the negated sub-lines carry the detail.
    let (credited_quantity, credited_unit_price) = if credit.sub_lines_overridden {
        (None, None)
    } else {
        effective_rate_display(item.quantity, credit_subtotal, precision)
    };
    // Per line: flagged prorated when the line was itself billed prorated, or —
    // for a proration-driven batch (`from_proration`, i.e. amendments) — when
    // proration actually reduced THIS line below what was billed. Within such a
    // batch the credit IS `billed × time_factor`, so `credit_subtotal < amount`
    // is exactly `factor < 1` (covers less than the full period). The
    // `from_proration` gate keeps manual partial credits (also a smaller amount,
    // but not a time-proration) from being mislabelled.
    let credited_is_prorated =
        item.is_prorated || (from_proration && credit_subtotal < item.amount_subtotal);

    CreditedLine {
        amount_subtotal: -credit_subtotal,
        taxable_amount: -prorated_taxable,
        tax_amount: -prorated_tax,
        amount_total: -credit_total,
        tax_details: prorated_tax_details,
        quantity: credited_quantity,
        unit_price: credited_unit_price,
        is_prorated: credited_is_prorated,
        credit_grant_amount: credited_grant_amount,
        promotional_credit: prorated_promotional,
        keeps_grant: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn detail(name: &str, rate: Decimal, tax_amount: i64) -> TaxDetail {
        TaxDetail {
            tax_rate: rate,
            tax_name: name.into(),
            tax_amount,
        }
    }

    #[test]
    fn partial_credits_of_a_credit_line_reverse_its_whole_grant_cumulatively() {
        let reversals = [0, 10, 20].map(|before| credit_grant_reversal(10, before, 10, 30));
        assert_eq!(reversals, [Some(3), Some(4), Some(3)]);
        assert_eq!(credit_grant_reversal(10, 0, 30, 30), Some(10));
        assert_eq!(credit_grant_reversal(10, 30, 1, 30), Some(0));
    }

    #[test]
    fn notes_completing_a_line_take_back_exactly_its_amount() {
        let halves = cumulative_share(333, 0, 500, 1_000) + cumulative_share(333, 500, 500, 1_000);
        assert_eq!(halves, 333);
        assert_eq!(cumulative_share(5, 0, 1, 2), 3);
        assert_eq!(cumulative_share(7, 0, 1, 0), 0);
    }

    /// The residual is spread within each detail's room: two details of 0.01 each with a line
    /// expecting nothing both return 0, where the largest alone would have charged 0.01 of tax.
    #[test]
    fn the_residual_spills_over_details_within_their_room() {
        let details = [
            detail("VAT", dec!(0.05), 40),
            detail("City", dec!(0.035), 28),
        ];
        let mut earlier = Returned::default();
        earlier
            .add(
                -500,
                -35,
                &[
                    detail("VAT", dec!(0.05), -21),
                    detail("City", dec!(0.035), -14),
                ],
            )
            .unwrap();
        let out = prorate_and_negate_tax_details(&details, (500, 50, 1_000), Some(&earlier), 0);
        let amounts: Vec<i64> = out.iter().map(|d| d.tax_amount).collect();
        assert_eq!(amounts, vec![0, 0]);
    }

    /// A line taxed 0.50 + 0.17 at two rates; a note of an earlier release returned 0.25 and 0.09
    /// for half of it. The note for the rest returns 0.25 and 0.08: the cent the rounding leaves
    /// never lands on the detail already returned beyond its share.
    #[test]
    fn the_residual_never_returns_more_of_a_detail_than_it_bore() {
        let line = BilledLine {
            amount_subtotal: 1_000,
            taxable_amount: 1_000,
            tax_amount: 67,
            tax_details: vec![
                detail("VAT", dec!(0.05), 50),
                detail("City", dec!(0.017), 17),
            ],
            quantity: Some(Decimal::ONE),
            unit_price: Some(dec!(10)),
            is_prorated: false,
            credit_grant_amount: None,
            promotional_credit: None,
        };
        let mut earlier = Returned::default();
        earlier
            .add(
                -500,
                -34,
                &[
                    detail("VAT", dec!(0.05), -25),
                    detail("City", dec!(0.017), -9),
                ],
            )
            .unwrap();
        let note = negate_line(
            &line,
            Credit {
                subtotal: Some(500),
                already_credited: 500,
                returned: Some(&earlier),
                sub_lines_overridden: false,
            },
            2,
            false,
        );
        assert_eq!(note.tax_amount, -33);
        let amounts: Vec<i64> = note.tax_details.iter().map(|d| d.tax_amount).collect();
        assert_eq!(amounts, vec![-25, -8], "0.50 and 0.17 returned in all");
    }

    #[test]
    fn a_line_billed_nothing_credits_nothing_and_no_grant() {
        let line = BilledLine {
            amount_subtotal: 0,
            taxable_amount: 0,
            tax_amount: 0,
            tax_details: vec![],
            quantity: Some(dec!(2)),
            unit_price: Some(Decimal::ZERO),
            is_prorated: false,
            credit_grant_amount: Some(100),
            promotional_credit: None,
        };
        let credit = Credit {
            subtotal: Some(0),
            already_credited: 0,
            returned: None,
            sub_lines_overridden: false,
        };
        let note = negate_line(&line, credit, 2, false);
        assert_eq!((note.amount_total, note.credit_grant_amount), (0, None));
        assert!(!note.keeps_grant);
    }
}
