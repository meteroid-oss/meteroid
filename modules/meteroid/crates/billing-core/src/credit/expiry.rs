//! What credit ending at a bound pays, before it lapses, of the usage consumed before it (R10,
//! D3): each open period's usage, net of what earlier settlements of it hold, drawn from the
//! grants as they stand just before the bound. Pure, so an ending writes these draws and a preview
//! folds the same draws over the grants it loaded.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use common_domain::ids::{CreditGrantId, SubscriptionId};

use crate::credit::allocation::{
    Allocation, Demand, Grant, Overflow, Pool, Request, allocate, assign,
};
use crate::credit::owed::UsageOwed;

/// One subscription period's usage up to the bound, by product.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeriodOwed {
    pub subscription: SubscriptionId,
    /// When its usage lines end: only credit effective before it pays them.
    pub dated_before: DateTime<Utc>,
    pub owed: Vec<UsageOwed>,
    /// What the grants that paid earlier settlements of the period still hold of them.
    pub held: Vec<(Grant, i64)>,
}

/// What one period's settlement draws, by grant in draw order, and the tax its paid draws carry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeriodDraws {
    pub draws: Vec<(CreditGrantId, i64)>,
    pub taxed: HashMap<CreditGrantId, i64>,
}

impl PeriodDraws {
    pub fn total(&self) -> Result<i64, Overflow> {
        self.draws
            .iter()
            .try_fold(0_i64, |sum, (_, amount)| sum.checked_add(*amount))
            .ok_or(Overflow)
    }
}

/// A period [`settle_expiry`] settles, with the grants whose draws on it are refused: their
/// wallet's settlement at the bound was taken over by an invoice, which marks the period settled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpiryPeriod {
    pub owed: PeriodOwed,
    pub refused: Vec<CreditGrantId>,
}

/// Whether `grant` ends at `bound`: one of `ending` (grants a change ends before their expiry),
/// or, when none is named, one lapsing then.
fn ends(grant: &Grant, ending: &[CreditGrantId], bound: DateTime<Utc>) -> bool {
    if ending.is_empty() {
        grant.lapses_at == Some(bound)
    } else {
        ending.contains(&grant.id)
    }
}

/// Whether credit ending at `bound`, with value spendable just before it, can pay `subscription`'s
/// usage dated before `dated_before`: a grant of that subscription, or a customer-wide one, whose
/// usage start is before then. Usage no ending credit can pay waits for billing order.
pub fn ending_pays(
    grants: &[Grant],
    ending: &[CreditGrantId],
    bound: DateTime<Utc>,
    subscription: SubscriptionId,
    dated_before: DateTime<Utc>,
) -> bool {
    grants.iter().any(|grant| {
        ends(grant, ending, bound)
            && grant.remaining_minor > 0
            && grant.effective_at < bound
            && grant.pays_usage_from < dated_before
            && grant
                .scope
                .subscription
                .is_none_or(|own| own == subscription)
    })
}

/// The draws paying `period`'s usage, net of what its earlier settlements hold, from `grants` as
/// they stand just before `bound`: those ending then first whatever their pool (what they leave
/// lapses), then, unless `ending` names the grants a change ends (only they pay), the others, each
/// set by `pools` in turn. Promotional credit pays usage before tax, paid credit with its tax.
pub fn settle_period(
    period: &PeriodOwed,
    grants: &[Grant],
    ending: &[CreditGrantId],
    bound: DateTime<Utc>,
    pools: &[Pool],
) -> Result<PeriodDraws, Overflow> {
    let at = bound - Duration::microseconds(1);
    let (expiring, mut others): (Vec<Grant>, Vec<Grant>) = grants
        .iter()
        .cloned()
        .partition(|grant| ends(grant, ending, bound));
    if !ending.is_empty() {
        others.clear();
    }
    let owed = &period.owed;
    let (subscription, dated_before) = (period.subscription, period.dated_before);
    let mut left: Vec<i64> = owed.iter().map(|owed| owed.base_minor.max(0)).collect();
    for pool in pools.iter().rev() {
        let held: Vec<(Grant, i64)> = period
            .held
            .iter()
            .filter(|(grant, _)| pool.admits(grant.classification))
            .cloned()
            .collect();
        let asked = owed_request(owed, &left, *pool, subscription, dated_before);
        let placed = assign(&held, &asked)?;
        pay_owed(owed, &mut left, *pool, &placed);
    }
    let mut taxed = HashMap::new();
    let mut plan = Allocation::default();
    for (set, pool) in [&expiring, &others]
        .into_iter()
        .flat_map(|set| pools.iter().map(move |pool| (set, *pool)))
    {
        let asked = owed_request(owed, &left, pool, subscription, dated_before);
        if asked.requested_minor <= 0 {
            continue;
        }
        let drawn = allocate(set, &asked, at, pool)?;
        if pool == Pool::Paid {
            carried_tax(owed, &left, &drawn, &mut taxed)?;
        }
        pay_owed(owed, &mut left, pool, &drawn);
        plan = plan.then(Allocation {
            draws: drawn.draws,
            covered: Vec::new(),
            placed: Vec::new(),
        })?;
    }
    Ok(PeriodDraws {
        draws: plan.draws,
        taxed,
    })
}

/// Settles `periods` in turn from `grants`, kept as the draws leave them: a period no credit
/// ending at `bound` can pay is skipped, and a refused grant's draws are dropped. Returns each
/// period's draws, in order.
pub fn settle_expiry(
    periods: &[ExpiryPeriod],
    grants: &mut [Grant],
    ending: &[CreditGrantId],
    bound: DateTime<Utc>,
    pools: &[Pool],
) -> Result<Vec<PeriodDraws>, Overflow> {
    let mut settled = Vec::with_capacity(periods.len());
    for period in periods {
        let owed = &period.owed;
        if !ending_pays(grants, ending, bound, owed.subscription, owed.dated_before) {
            settled.push(PeriodDraws::default());
            continue;
        }
        let mut drawn = settle_period(owed, grants, ending, bound, pools)?;
        drawn.draws.retain(|(id, _)| !period.refused.contains(id));
        drawn.taxed.retain(|id, _| !period.refused.contains(id));
        for (id, amount) in &drawn.draws {
            if let Some(grant) = grants.iter_mut().find(|grant| grant.id == *id) {
                grant.remaining_minor =
                    grant.remaining_minor.checked_sub(*amount).ok_or(Overflow)?;
            }
        }
        settled.push(drawn);
    }
    Ok(settled)
}

/// What `owed` (`left` of it) asks of `pool`: promotional credit pays its usage before tax, paid
/// credit its invoiced figure.
fn owed_request(
    owed: &[UsageOwed],
    left: &[i64],
    pool: Pool,
    subscription: SubscriptionId,
    dated_before: DateTime<Utc>,
) -> Request {
    let amounts = if pool == Pool::Paid {
        UsageOwed::invoiced_together(owed, left)
    } else {
        left.to_vec()
    };
    Request {
        requested_minor: amounts.iter().sum(),
        demands: owed
            .iter()
            .zip(amounts)
            .map(|(owed, amount)| Demand {
                subscription: Some(subscription),
                product: owed.product,
                usage: true,
                amount_minor: amount,
                dated_before: Some(dated_before),
            })
            .collect(),
    }
}

/// Takes off `left` the usage `paid` (drawn from `pool`) pays: paid credit covers invoiced
/// figures, so what it pays comes off the usage they invoice.
fn pay_owed(owed: &[UsageOwed], left: &mut [i64], pool: Pool, paid: &Allocation) {
    let full = UsageOwed::invoiced_together(owed, left);
    for (index, base) in left.iter_mut().enumerate() {
        let covered = paid.covered.get(index).copied().unwrap_or(0);
        *base = if pool == Pool::Paid {
            *base - owed[index].usage_paid_by_within(*base, covered, full[index])
        } else {
            (*base - covered).max(0)
        };
    }
}

/// Adds the tax the paid credit `drawn` carries to its draws, split by what each drew so the
/// parts sum to it exactly.
fn carried_tax(
    owed: &[UsageOwed],
    left: &[i64],
    drawn: &Allocation,
    taxed: &mut HashMap<CreditGrantId, i64>,
) -> Result<(), Overflow> {
    let full = UsageOwed::invoiced_together(owed, left);
    let tax = owed
        .iter()
        .zip(left)
        .enumerate()
        .try_fold(0_i64, |sum, (index, (owed, base))| {
            sum.checked_add(owed.tax_in_within(
                *base,
                drawn.covered.get(index).copied().unwrap_or(0),
                full[index],
            ))
        })
        .ok_or(Overflow)?;
    let total = drawn.total()?;
    if tax == 0 || total <= 0 {
        return Ok(());
    }
    let (mut cumulative, mut assigned) = (0_i128, 0_i64);
    for (grant_id, amount) in &drawn.draws {
        cumulative += i128::from(*amount);
        let share = i64::try_from(cumulative * i128::from(tax) / i128::from(total))
            .map_err(|_| Overflow)?;
        *taxed.entry(*grant_id).or_default() += share - assigned;
        assigned = share;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credit::allocation::{Application, Classification, Scope};
    use crate::document::Rate;
    use chrono::TimeZone;
    use proptest::prelude::*;
    use rust_decimal_macros::dec;
    use std::sync::atomic::{AtomicU64, Ordering as Atomic};
    use uuid::Uuid;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap()
    }

    /// Ids as random as the store's: no test may lean on their order.
    fn id<T: From<Uuid>>() -> T {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Atomic::Relaxed);
        Uuid::from_u128(u128::from(n).wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835))
            .into()
    }

    /// A grant of `subscription` funded on Jan 1, lapsing at the period's end on Feb 1.
    fn grant(subscription: SubscriptionId, classification: Classification, left: i64) -> Grant {
        Grant {
            id: id(),
            scope: Scope {
                subscription: Some(subscription),
                application: Application::Invoice,
                products: Vec::new(),
            },
            remaining_minor: left,
            priority: 100,
            classification,
            effective_at: at(1),
            pays_usage_from: at(1),
            lapses_at: Some(at(31) + Duration::days(1)),
            created_at: at(1),
        }
    }

    fn usage(base_minor: i64, taxes: Vec<Rate>) -> UsageOwed {
        UsageOwed {
            product: None,
            base_minor,
            tax_inclusive: false,
            taxes,
        }
    }

    fn period(subscription: SubscriptionId, owed: Vec<UsageOwed>) -> PeriodOwed {
        PeriodOwed {
            subscription,
            dated_before: at(31) + Duration::days(1),
            owed,
            held: Vec::new(),
        }
    }

    const MONEY: [Pool; 2] = [Pool::Promotional, Pool::Paid];

    /// Scale -> Studio on Jan 16 after 350.00 of usage: the change ends the 500.00 pack and its
    /// 50.00 bonus, the bonus pays 50.00 first, the paid credit the other 300.00; a welcome credit
    /// the change does not end pays nothing. 200.00 of paid credit is left to give back.
    /// Rules: D3, OQ1
    #[test]
    fn a_plan_change_settles_its_usage_from_the_packs_it_ends_only() {
        let subscription = id();
        let paid = grant(subscription, Classification::Paid, 50_000);
        let bonus = grant(subscription, Classification::Promotional, 5_000);
        let welcome = Grant {
            lapses_at: None,
            ..grant(subscription, Classification::Promotional, 5_000)
        };
        let mut grants = vec![welcome.clone(), paid.clone(), bonus.clone()];
        let ending = [paid.id, bonus.id];
        let periods = [ExpiryPeriod {
            owed: period(subscription, vec![usage(35_000, Vec::new())]),
            refused: Vec::new(),
        }];
        let drawn = settle_expiry(&periods, &mut grants, &ending, at(16), &MONEY).unwrap();
        assert_eq!(drawn[0].draws, vec![(bonus.id, 5_000), (paid.id, 30_000)]);
        let left: Vec<i64> = grants.iter().map(|grant| grant.remaining_minor).collect();
        assert_eq!(
            left,
            vec![5_000, 20_000, 0],
            "the welcome credit is untouched"
        );
    }

    /// The same usage taxed at 20%: the bonus pays 50.00 before tax, the paid credit the other
    /// 300.00 with its 60.00 of tax, which it carries.
    /// Rules: D3
    #[test]
    fn paid_credit_pays_usage_with_its_tax() {
        let subscription = id();
        let paid = grant(subscription, Classification::Paid, 50_000);
        let bonus = grant(subscription, Classification::Promotional, 5_000);
        let vat = vec![Rate {
            rate: dec!(0.20),
            name: "VAT".to_owned(),
        }];
        let drawn = settle_period(
            &period(subscription, vec![usage(35_000, vat)]),
            &[paid.clone(), bonus.clone()],
            &[paid.id, bonus.id],
            at(16),
            &MONEY,
        )
        .unwrap();
        assert_eq!(drawn.draws, vec![(bonus.id, 5_000), (paid.id, 36_000)]);
        assert_eq!(drawn.taxed.get(&paid.id), Some(&6_000));
        assert_eq!(drawn.total(), Ok(41_000));
    }

    /// $54 of paid credit settling usage taxed 10% on top pays $49.09 of it and carries its $4.91
    /// of tax, split exactly over the grants that drew it.
    /// Rules: D3
    #[test]
    fn the_tax_carried_is_split_exactly_over_the_grants_that_drew_it() {
        let ten = UsageOwed {
            taxes: vec![Rate {
                rate: dec!(0.10),
                name: "Sales tax".to_owned(),
            }],
            ..usage(4_909, Vec::new())
        };
        let (first, second) = (id(), id());
        let drawn = Allocation {
            draws: vec![(first, 3_000), (second, 2_400)],
            covered: vec![5_400],
            placed: Vec::new(),
        };
        let mut taxed = HashMap::new();
        carried_tax(&[ten], &[4_909], &drawn, &mut taxed).unwrap();
        assert_eq!((taxed[&first], taxed[&second]), (272, 219));
    }

    /// What an earlier settlement of the period holds is not paid again: of 350.00, 100.00 was
    /// settled by the paid credit at an earlier bound, so the ending pays the other 250.00.
    /// Rules: R10
    #[test]
    fn an_earlier_settlement_of_the_period_is_not_paid_again() {
        let subscription = id();
        let paid = grant(subscription, Classification::Paid, 40_000);
        let mut owed = period(subscription, vec![usage(35_000, Vec::new())]);
        owed.held = vec![(paid.clone(), 10_000)];
        let drawn = settle_period(
            &owed,
            std::slice::from_ref(&paid),
            &[paid.id],
            at(16),
            &MONEY,
        )
        .unwrap();
        assert_eq!(drawn.draws, vec![(paid.id, 25_000)]);
    }

    /// Credit that took effect after the bound, of another subscription, or already spent
    /// settles nothing: the period is skipped and nothing is drawn.
    /// Rules: D3
    #[test]
    fn credit_not_spendable_before_the_bound_settles_nothing() {
        let subscription = id();
        let late = Grant {
            effective_at: at(16) + Duration::hours(14),
            ..grant(subscription, Classification::Paid, 50_000)
        };
        let other = grant(id(), Classification::Paid, 50_000);
        let spent = grant(subscription, Classification::Paid, 0);
        let mut grants = vec![late.clone(), other.clone(), spent.clone()];
        let ending = [late.id, other.id, spent.id];
        assert!(!ending_pays(
            &grants,
            &ending,
            at(16),
            subscription,
            at(31) + Duration::days(1)
        ));
        let periods = [ExpiryPeriod {
            owed: period(subscription, vec![usage(35_000, Vec::new())]),
            refused: Vec::new(),
        }];
        let drawn = settle_expiry(&periods, &mut grants, &ending, at(16), &MONEY).unwrap();
        assert_eq!(drawn, vec![PeriodDraws::default()]);
        assert_eq!(grants[0].remaining_minor, 50_000);
    }

    /// A wallet whose settlement at the bound an invoice took over draws nothing more, and its
    /// grants keep their value for the next period.
    /// Rules: R10
    #[test]
    fn a_refused_wallet_draws_nothing() {
        let subscription = id();
        let paid = grant(subscription, Classification::Paid, 50_000);
        let mut grants = vec![paid.clone()];
        let periods = [ExpiryPeriod {
            owed: period(subscription, vec![usage(35_000, Vec::new())]),
            refused: vec![paid.id],
        }];
        let drawn = settle_expiry(&periods, &mut grants, &[paid.id], at(16), &MONEY).unwrap();
        assert!(drawn[0].draws.is_empty());
        assert_eq!(grants[0].remaining_minor, 50_000);
    }

    /// Without named grants, the credit lapsing at the bound pays first, then the rest in draw
    /// order: a period lapse spends the lapsing grant before one that lives on.
    /// Rules: D3
    #[test]
    fn a_lapse_pays_from_the_lapsing_credit_first() {
        let subscription = id();
        let lasting = Grant {
            lapses_at: None,
            ..grant(subscription, Classification::Paid, 50_000)
        };
        let lapsing = Grant {
            lapses_at: Some(at(16)),
            ..grant(subscription, Classification::Paid, 10_000)
        };
        let drawn = settle_period(
            &period(subscription, vec![usage(35_000, Vec::new())]),
            &[lasting.clone(), lapsing.clone()],
            &[],
            at(16),
            &MONEY,
        )
        .unwrap();
        assert_eq!(
            drawn.draws,
            vec![(lapsing.id, 10_000), (lasting.id, 25_000)]
        );
    }

    proptest! {
        /// Whatever the usage, the earlier settlements and the grants, a settlement never draws
        /// more than a grant holds nor more than the usage owes with its tax, and folding it
        /// leaves no grant below zero.
        /// Rules: D3, R10
        #[test]
        fn draws_never_exceed_grants_or_owed(
            owed in prop::collection::vec((0i64..10_000_000, any::<bool>()), 1..4),
            grants in prop::collection::vec((-1_000i64..10_000_000, any::<bool>(), any::<bool>()), 0..5),
            held in 0i64..1_000_000,
            periods in 1usize..3,
        ) {
            let subscription = id();
            let owed: Vec<UsageOwed> = owed
                .into_iter()
                .map(|(base, taxed)| {
                    let taxes = if taxed {
                        vec![Rate { rate: dec!(0.20), name: "VAT".to_owned() }]
                    } else {
                        Vec::new()
                    };
                    usage(base, taxes)
                })
                .collect();
            let mut pool: Vec<Grant> = grants
                .into_iter()
                .map(|(left, paid, ended)| Grant {
                    lapses_at: ended.then(|| at(16)),
                    ..grant(
                        subscription,
                        if paid { Classification::Paid } else { Classification::Promotional },
                        left,
                    )
                })
                .collect();
            let before = pool.clone();
            let ending: Vec<CreditGrantId> = Vec::new();
            let mut first = period(subscription, owed.clone());
            if let Some(holder) = before.first() {
                first.held = vec![(holder.clone(), held)];
            }
            let all: Vec<ExpiryPeriod> = (0..periods)
                .map(|_| ExpiryPeriod { owed: first.clone(), refused: Vec::new() })
                .collect();
            let bases: Vec<i64> = owed.iter().map(|owed| owed.base_minor.max(0)).collect();
            let invoiced: i64 = UsageOwed::invoiced_together(&owed, &bases).iter().sum();
            let drawn = settle_expiry(&all, &mut pool, &ending, at(16), &MONEY).unwrap();
            for draws in &drawn {
                prop_assert!(draws.total().unwrap() <= invoiced);
                for (_, amount) in &draws.draws {
                    prop_assert!(*amount > 0);
                }
            }
            for (after, was) in pool.iter().zip(&before) {
                let taken: i64 = drawn
                    .iter()
                    .flat_map(|draws| draws.draws.iter())
                    .filter(|(id, _)| *id == was.id)
                    .map(|(_, amount)| *amount)
                    .sum();
                prop_assert!(taken <= was.remaining_minor.max(0));
                prop_assert_eq!(after.remaining_minor, was.remaining_minor - taken);
                prop_assert!(after.remaining_minor >= 0 || was.remaining_minor < 0);
            }
        }
    }
}
