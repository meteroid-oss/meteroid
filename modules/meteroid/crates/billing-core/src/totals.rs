//! A document's totals, folded from its lines the same way for every kind of document (R-076).

/// What one line adds to its document's totals, signed: a credit line subtracts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineAmounts {
    pub subtotal: i64,
    pub tax: i64,
    /// Promotional credit taken off the line before tax.
    pub promotional: i64,
    /// A line billing no metered usage.
    pub recurring: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub discount: i64,
    pub promotional: i64,
    pub tax: i64,
    /// `subtotal - discount - promotional + tax`.
    pub total: i64,
}

impl Totals {
    /// The totals of `lines` under the document's `discount`, already spread over their taxable
    /// amounts.
    pub fn of(lines: impl IntoIterator<Item = LineAmounts>, discount: i64) -> Totals {
        let mut totals = Totals {
            discount,
            ..Totals::default()
        };
        for line in lines {
            totals.subtotal += line.subtotal;
            totals.tax += line.tax;
            totals.promotional += line.promotional;
            if line.recurring {
                totals.subtotal_recurring += line.subtotal;
            }
        }
        totals.total = totals.subtotal - totals.discount - totals.promotional + totals.tax;
        totals
    }

    /// What the customer still owes once `already_paid` and `applied_credit` are taken off: never
    /// negative.
    pub fn amount_due(&self, already_paid: i64, applied_credit: i64) -> i64 {
        amount_due(self.total, already_paid, applied_credit)
    }
}

/// What the customer still owes on `total` once `already_paid` and `applied_credit` are taken off:
/// never negative.
pub fn amount_due(total: i64, already_paid: i64, applied_credit: i64) -> i64 {
    total
        .saturating_sub(already_paid)
        .saturating_sub(applied_credit)
        .max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn line(subtotal: i64, tax: i64, promotional: i64, recurring: bool) -> LineAmounts {
        LineAmounts {
            subtotal,
            tax,
            promotional,
            recurring,
        }
    }

    #[test]
    fn a_document_adds_up_its_lines() {
        let totals = Totals::of(
            [line(10_000, 1_800, 1_000, true), line(500, 100, 0, false)],
            500,
        );
        assert_eq!(
            totals,
            Totals {
                subtotal: 10_500,
                subtotal_recurring: 10_000,
                discount: 500,
                promotional: 1_000,
                tax: 1_900,
                total: 10_900,
            }
        );
        assert_eq!(totals.amount_due(900, 2_000), 8_000);
        assert_eq!(totals.amount_due(10_000, 2_000), 0, "never negative");
    }

    proptest! {
        #[test]
        fn the_total_is_the_identity_of_its_parts(
            amounts in prop::collection::vec((-1_000_000i64..1_000_000, 0i64..100_000, 0i64..100_000, any::<bool>()), 0..20),
            discount in 0i64..1_000_000,
            paid in 0i64..1_000_000,
            credit in 0i64..1_000_000,
        ) {
            let totals = Totals::of(amounts.iter().map(|&(s, t, p, r)| line(s, t, p, r)), discount);
            prop_assert_eq!(totals.total, totals.subtotal - totals.discount - totals.promotional + totals.tax);
            prop_assert!(totals.amount_due(paid, credit) >= 0);
            prop_assert!(totals.amount_due(paid, credit) <= totals.total.max(0));
        }
    }
}
