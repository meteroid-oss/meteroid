//! The money behind credit: what each grant cost, and the share of it each ledger movement
//! carries. A grant's cost is split cumulatively over what it no longer holds, rounded once, so
//! its entries always sum exactly to its cost whatever the order and size of the movements.
use crate::StoreResult;
use crate::domain::credit::{CreditCost, GrantClassification};
use crate::domain::credit_units::DenominationCode;
use crate::repositories::credit_notes::cumulative_grant_reversal;
use crate::services::credit::{invalid, precondition};
use diesel_models::customer_credit::CustomerCreditGrantRow;

/// `C(x)`: cost of the first `consumed` minor units of a grant of `amount` that cost `cost`,
/// rounded half up.
fn cost_of_consumed(amount: i64, cost: i64, consumed: i64) -> StoreResult<i64> {
    if amount <= 0 || cost < 0 || !(0..=amount).contains(&consumed) {
        return Err(precondition(format!(
            "a credit grant of {amount} cannot have consumed {consumed}"
        )));
    }
    let (amount, cost, consumed) = (i128::from(amount), i128::from(cost), i128::from(consumed));
    let rounded = (2 * consumed * cost + amount) / (2 * amount);
    i64::try_from(rounded).map_err(|_| invalid("credit cost exceeds the supported range"))
}

/// Cost carried by a movement of `delta` (signed like the entry) that left a grant of `amount`
/// and total `cost` holding `remaining_after`. Same sign as `delta`, or zero.
pub(crate) fn movement_cost(
    amount: i64,
    cost: i64,
    remaining_after: i64,
    delta: i64,
) -> StoreResult<i64> {
    let consumed_after = amount
        .checked_sub(remaining_after)
        .ok_or_else(|| invalid("credit cost exceeds the supported range"))?;
    let consumed_before = consumed_after
        .checked_add(delta)
        .ok_or_else(|| invalid("credit cost exceeds the supported range"))?;
    Ok(cost_of_consumed(amount, cost, consumed_before)?
        - cost_of_consumed(amount, cost, consumed_after)?)
}

/// The cost of the entry recording `delta` on `grant`, as the grant stands after it.
pub(crate) fn entry_cost(grant: &CustomerCreditGrantRow, delta: i64) -> StoreResult<Option<i64>> {
    grant
        .cost_minor
        .map(|cost| movement_cost(grant.amount, cost, grant.remaining, delta))
        .transpose()
}

/// The cost `carried` minor units taken from `grant` (as it stands before) carry to a new grant:
/// exactly what the entry taking them records, with the opposite sign. Used by rollover.
pub(crate) fn carried_cost(
    grant: &CustomerCreditGrantRow,
    carried: i64,
) -> StoreResult<Option<CreditCost>> {
    let (Some(cost), Some(currency)) = (grant.cost_minor, grant.cost_currency.as_deref()) else {
        return Ok(None);
    };
    if carried <= 0 || carried > grant.remaining {
        return Err(precondition(format!(
            "{carried} cannot be carried from a credit grant holding {}",
            grant.remaining
        )));
    }
    let taken = movement_cost(grant.amount, cost, grant.remaining - carried, -carried)?;
    Ok(Some(CreditCost::at_face(-taken, currency)))
}

/// What a paid `Credit` line's grant cost: the line's amount net of discounts and tax, less the
/// share finalized credit notes took off it (`credited` of its `subtotal`), prorated like the
/// credit those notes cancelled.
pub(crate) fn purchased_cost(
    taxable_amount: i64,
    subtotal: i64,
    credited: i64,
    currency: &str,
) -> StoreResult<CreditCost> {
    if taxable_amount < 0 {
        return Err(invalid("a credit purchase line cannot be negative"));
    }
    let cancelled = match credited {
        0 => 0,
        credited => cumulative_grant_reversal(taxable_amount, credited, subtotal)
            .ok_or_else(|| invalid("the credit notes of this purchase line cannot be prorated"))?,
    };
    Ok(CreditCost::at_face(taxable_amount - cancelled, currency))
}

/// What a grant made by hand cost: the stated cost when given, otherwise nothing for promotional
/// credit and its face for paid credit in a currency. Credit units have no face value in money.
pub(crate) fn manual_grant_cost(
    classification: GrantClassification,
    amount_minor: i64,
    wallet_currency: &str,
    stated: Option<CreditCost>,
) -> StoreResult<CreditCost> {
    if let Some(stated) = stated {
        if stated.amount_minor < 0 {
            return Err(invalid("a credit grant's cost cannot be negative"));
        }
        if !matches!(
            DenominationCode::parse(&stated.currency),
            Ok(DenominationCode::Currency(_))
        ) {
            return Err(invalid(format!(
                "a credit grant's cost is in a currency, not {}",
                stated.currency
            )));
        }
        if classification == GrantClassification::Promotional && stated.amount_minor > 0 {
            return Err(invalid("promotional credit costs nothing"));
        }
        return Ok(CreditCost::at_face(stated.amount_minor, &stated.currency));
    }
    match DenominationCode::parse(wallet_currency) {
        Ok(DenominationCode::Currency(currency)) => Ok(match classification {
            GrantClassification::Promotional => CreditCost::given(&currency),
            GrantClassification::Paid => CreditCost::at_face(amount_minor, &currency),
        }),
        _ => Err(invalid(
            "a grant of credit units needs the cost it was given or paid for",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift, so a failing sequence replays.
    struct Sequence(u64);

    impl Sequence {
        fn below(&mut self, bound: i64) -> i64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            i64::try_from(self.0 % u64::try_from(bound).unwrap()).unwrap()
        }
    }

    /// Replays signed movements on a grant, checking each entry and the running conservation;
    /// returns the entries' costs, FUND first.
    fn replay(amount: i64, cost: i64, movements: &[i64]) -> Vec<i64> {
        let mut costs = vec![movement_cost(amount, cost, amount, amount).unwrap()];
        let mut remaining = amount;
        for &delta in movements {
            remaining += delta;
            let moved = movement_cost(amount, cost, remaining, delta).unwrap();
            assert!(
                moved == 0 || moved.signum() == delta.signum(),
                "{delta} carried {moved}"
            );
            costs.push(moved);
            let held: i64 = costs.iter().sum();
            assert_eq!(
                held,
                cost - cost_of_consumed(amount, cost, amount - remaining).unwrap(),
                "the entries hold the cost of what the grant still holds"
            );
        }
        costs
    }

    #[test]
    fn three_credits_for_ten_split_exactly_over_three_draws() {
        assert_eq!(
            replay(3, 1_000, &[-1, -1, -1]),
            vec![1_000, -333, -334, -333]
        );
    }

    #[test]
    fn odd_draws_of_a_discounted_pack_sum_to_its_cost() {
        let costs = replay(1_000, 800, &[-333, -333, -334]);
        assert_eq!(costs, vec![800, -266, -267, -267]);
        assert_eq!(costs[1..].iter().sum::<i64>(), -800);
    }

    #[test]
    fn a_draw_returned_and_drawn_again_carries_the_same_cost() {
        let costs = replay(7, 10, &[-3, 3, -3, -4]);
        assert_eq!(costs, vec![10, -4, 4, -4, -6]);
    }

    #[test]
    fn expiry_takes_the_unconsumed_cost_as_breakage() {
        let costs = replay(10_000, 4_000, &[-2_501, -7_499]);
        assert_eq!(costs, vec![4_000, -1_000, -3_000]);
    }

    #[test]
    fn refunds_reverse_the_cost_cumulatively() {
        // A pack of 10 sold for 30, refunded in three notes taking 3, 4 and 3 back.
        assert_eq!(replay(10, 30, &[-3, -4, -3]), vec![30, -9, -12, -9]);
    }

    #[test]
    fn given_credit_carries_no_cost() {
        assert_eq!(replay(500, 0, &[-200, -300]), vec![0, 0, 0]);
    }

    #[test]
    fn random_movements_always_sum_to_the_cost_of_what_was_consumed() {
        let mut sequence = Sequence(0x9E37_79B9_7F4A_7C15);
        for _ in 0..2_000 {
            let amount = 1 + sequence.below(10_000);
            let cost = sequence.below(3 * amount + 7);
            let mut remaining = amount;
            let mut movements = Vec::new();
            for _ in 0..sequence.below(40) {
                let delta = if remaining > 0 && (remaining == amount || sequence.below(3) > 0) {
                    -(1 + sequence.below(remaining))
                } else {
                    1 + sequence.below(amount - remaining)
                };
                remaining += delta;
                movements.push(delta);
            }
            // Whatever is left lapses: the grant is fully consumed and its cost fully carried.
            if remaining > 0 {
                movements.push(-remaining);
            }
            let costs = replay(amount, cost, &movements);
            assert_eq!(
                costs[1..].iter().sum::<i64>(),
                -cost,
                "{amount} for {cost}: {movements:?}"
            );
        }
    }

    #[test]
    fn extreme_amounts_never_overflow() {
        let max = i64::MAX;
        assert_eq!(movement_cost(max, max, 0, -max).unwrap(), -max);
        assert_eq!(movement_cost(max, max, max - 1, -1).unwrap(), -1);
        assert!(
            movement_cost(10, 5, 11, 1).is_err(),
            "a grant never holds more than it was funded"
        );
        assert!(movement_cost(10, 5, 0, -11).is_err());
    }

    fn grant(amount: i64, remaining: i64, cost: Option<i64>) -> CustomerCreditGrantRow {
        use chrono::Utc;
        use common_domain::ids::{BaseId, CreditGrantId, CustomerId, TenantId, WalletId};
        CustomerCreditGrantRow {
            id: CreditGrantId::new(),
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            account_id: WalletId::new(),
            amount,
            remaining,
            priority: 100,
            classification: "PROMOTIONAL".to_owned(),
            effective_at: Utc::now(),
            expires_at: None,
            source_kind: "INCLUDED_CREDIT".to_owned(),
            source_id: uuid::Uuid::nil(),
            invoice_id: None,
            line_local_id: None,
            credit_note_id: None,
            reason: "included".to_owned(),
            reference: None,
            created_by: None,
            created_at: Utc::now(),
            cost_minor: cost,
            cost_currency: cost.map(|_| "USD".to_owned()),
            rollover: None,
            subscription_id: None,
            application: "USAGE".to_owned(),
            ends_at: None,
            end_reason: None,
            lapses_at: None,
            pays_usage_from: None,
        }
    }

    #[test]
    fn a_rolled_share_carries_what_its_entry_takes_and_the_rest_lapses_as_breakage() {
        // 1,000 for 300, 400 drawn; 250 of the 600 left roll over, 350 lapse.
        let before = grant(1_000, 600, Some(300));
        let carried = carried_cost(&before, 250).unwrap().unwrap();
        assert_eq!(carried, CreditCost::at_face(75, "USD"));
        let rolled_entry = movement_cost(1_000, 300, 350, -250).unwrap();
        assert_eq!(rolled_entry, -carried.amount_minor);
        let lapsed = movement_cost(1_000, 300, 0, -350).unwrap();
        assert_eq!(
            -120 + rolled_entry + lapsed,
            -300,
            "draws, rollover and expiry carry it all"
        );
        assert_eq!(carried_cost(&grant(1_000, 600, None), 250).unwrap(), None);
        assert!(carried_cost(&before, 601).is_err());
        assert_eq!(entry_cost(&grant(1_000, 600, None), -400).unwrap(), None);
        assert_eq!(
            entry_cost(&grant(1_000, 600, Some(300)), -400).unwrap(),
            Some(-120)
        );
    }

    #[test]
    fn a_purchase_costs_its_net_amount_less_what_notes_cancelled() {
        // A 5,000 pack discounted to 4,000; nothing credited.
        assert_eq!(
            purchased_cost(4_000, 5_000, 0, "usd").unwrap(),
            CreditCost::at_face(4_000, "USD")
        );
        // 6,000 discounted to 4,000, a third of its subtotal credited: a third of the net, rounded once.
        assert_eq!(
            purchased_cost(4_000, 6_000, 2_000, "USD").unwrap(),
            CreditCost::at_face(2_667, "USD")
        );
        assert_eq!(
            purchased_cost(0, 5_000, 0, "USD").unwrap(),
            CreditCost::given("USD"),
            "credit bought for nothing was given"
        );
        assert!(purchased_cost(-1, 5_000, 0, "USD").is_err());
    }

    #[test]
    fn a_manual_grant_costs_its_face_when_paid_and_nothing_when_given() {
        use GrantClassification::{Paid, Promotional};
        assert_eq!(
            manual_grant_cost(Paid, 2_500, "EUR", None).unwrap(),
            CreditCost::at_face(2_500, "EUR")
        );
        assert_eq!(
            manual_grant_cost(Promotional, 2_500, "EUR", None).unwrap(),
            CreditCost::given("EUR")
        );
        let stated = CreditCost::at_face(1_999, "USD");
        assert_eq!(
            manual_grant_cost(Paid, 2_500, "CREDITS", Some(stated.clone())).unwrap(),
            stated
        );
        assert!(manual_grant_cost(Paid, 2_500, "CREDITS", None).is_err());
        assert!(manual_grant_cost(Promotional, 2_500, "EUR", Some(stated)).is_err());
        assert!(
            manual_grant_cost(Paid, 2_500, "EUR", Some(CreditCost::at_face(1, "CREDITS"))).is_err()
        );
        assert!(
            manual_grant_cost(Paid, 2_500, "EUR", Some(CreditCost::at_face(-1, "EUR"))).is_err()
        );
    }

    #[test]
    fn the_cost_basis_is_the_price_of_one_whole_unit() {
        let pack = CreditCost::at_face(4_000, "USD");
        assert_eq!(pack.per_unit(500_000, 2).unwrap().to_string(), "0.008");
        assert_eq!(
            CreditCost::at_face(1_000, "USD")
                .per_unit(3, 0)
                .unwrap()
                .to_string(),
            "3.333333333333"
        );
        assert_eq!(CreditCost::at_face(1, "XXX_").per_unit(3, 0), None);
    }
}
