//! Consolidation: what each merged member takes of what its consolidated invoice settles.

use rust_decimal::Decimal;

use crate::allocation::split;

/// Each member's part of the credit applied to the consolidated invoice, split (R-003) by what
/// each still owes (`totals` less `already_paid`), so no member's amount due clamps and loses
/// what it collected.
pub fn applied_credit_shares(
    totals: &[i64],
    already_paid: &[i64],
    applied_credits: i64,
) -> Vec<i64> {
    let weights: Vec<Decimal> = totals
        .iter()
        .zip(already_paid)
        .map(|(total, paid)| Decimal::from((total - paid).max(0)))
        .collect();
    split(applied_credits, &weights)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_take_the_credit_by_what_they_still_owe() {
        assert_eq!(
            applied_credit_shares(&[1_000, 3_000], &[0, 2_000], 1_000),
            vec![500, 500]
        );
        assert_eq!(
            applied_credit_shares(&[1_000, 500], &[1_000, 0], 300),
            vec![0, 300]
        );
    }
}
