//! Usage paid ahead of its invoice: what a settlement pays tax included, where the settlements an
//! invoice keeps land (R10), and what promotional credit may still take once they are placed.

use chrono::{NaiveDate, NaiveTime};
use uuid::Uuid;

use crate::credit::SettleError;
use crate::credit::allocation::{Allocation, Demand, Grant, Overflow, Request, assign};
use crate::document::Rate;
use crate::tax::{LineToTax, TaxToApply, apply_taxes};

/// What a settlement pays for one product: its usage before tax, and the taxes its invoice line
/// will levy. Paid credit pays the usage with its tax; promotional credit comes off before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageOwed {
    pub product: Option<Uuid>,
    pub base_minor: i64,
    pub tax_inclusive: bool,
    pub taxes: Vec<Rate>,
}

impl UsageOwed {
    /// What `base` of it is invoiced at, tax included.
    pub fn invoiced(&self, base: i64) -> i64 {
        if self.tax_inclusive || self.taxes.is_empty() || base <= 0 {
            return base;
        }
        base + self.levied(base, false)
    }

    /// The tax in what `covered` of `base`'s invoiced figure pays, as its invoice line levies it,
    /// `base` being invoiced at `full` (`invoiced_together`).
    pub fn tax_in_within(&self, base: i64, covered: i64, full: i64) -> i64 {
        if self.taxes.is_empty() || covered <= 0 {
            0
        } else if self.tax_inclusive {
            self.levied(covered, true)
        } else if covered >= full {
            full - base.max(0)
        } else {
            let usage = self.usage_paid_by_within(base, covered, full);
            self.invoiced(usage) - usage
        }
    }

    fn tax_keys(&self) -> Vec<TaxToApply> {
        self.taxes
            .iter()
            .map(|tax| TaxToApply {
                rate: tax.rate,
                key: format!("{}:{}", tax.name, tax.rate),
                compound: false,
            })
            .collect()
    }

    fn levied(&self, amount: i64, tax_inclusive: bool) -> i64 {
        let line = LineToTax {
            taxable_amount: amount,
            tax_inclusive,
            taxes: self.tax_keys(),
        };
        apply_taxes(&[line]).into_iter().flatten().sum::<i64>()
    }

    /// What each of `owed` is invoiced at for `bases`, tax included, each rate rounded once over
    /// all of them as the invoice taking them over rounds it (OQ14), not line by line.
    pub fn invoiced_together(owed: &[Self], bases: &[i64]) -> Vec<i64> {
        let lines: Vec<LineToTax> = owed
            .iter()
            .zip(bases)
            .map(|(owed, base)| LineToTax {
                taxable_amount: if owed.tax_inclusive {
                    0
                } else {
                    (*base).max(0)
                },
                tax_inclusive: false,
                taxes: owed.tax_keys(),
            })
            .collect();
        apply_taxes(&lines)
            .into_iter()
            .zip(bases)
            .map(|(taxes, base)| base + taxes.iter().sum::<i64>())
            .collect()
    }

    /// `usage_paid_by` where `base` is invoiced at `full`: all of it only once `paid` reaches it.
    pub fn usage_paid_by_within(&self, base: i64, paid: i64, full: i64) -> i64 {
        let base = base.max(0);
        if paid >= full {
            return base;
        }
        self.usage_paid_by(base, paid).min((base - 1).max(0))
    }

    /// The usage before tax that `paid` of `base`'s invoiced figure pays: all of it once `paid`
    /// reaches that figure, else the most whose invoiced figure `paid` covers. Not every amount is
    /// an invoiced figure (3 at 20% invoices at 4), so up to one minor unit per tax pays no usage.
    pub fn usage_paid_by(&self, base: i64, paid: i64) -> i64 {
        let base = base.max(0);
        if paid <= 0 {
            return 0;
        }
        if paid >= self.invoiced(base) {
            return base;
        }
        let (mut low, mut high) = (0_i64, base);
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            if self.invoiced(middle) <= paid {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    }
}

/// A usage charge of a promotional request (`demand`), with the rates its invoice line levies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageCharge {
    pub demand: usize,
    pub owed: UsageOwed,
}

/// The usage window a settlement paid: its subscription's period starting on that day.
pub type Window = (Uuid, NaiveDate);

/// What a grant still holds of the usage settlements it paid, by the window each paid (None: what
/// no settlement accounts for).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settled {
    pub window: Option<Window>,
    pub grant: Grant,
    pub amount: i64,
}

/// Places settled credit on `request`, each settlement's on the usage of the window it paid only,
/// never on an earlier window another grant could not pay (R10). Merged per grant.
pub fn assign_settled(held: &[Settled], request: &Request) -> Result<Allocation, Overflow> {
    let mut kept = Allocation {
        draws: Vec::new(),
        covered: vec![0; request.demands.len()],
        placed: Vec::new(),
    };
    let mut windows: Vec<Option<Window>> = Vec::new();
    for settled in held {
        if !windows.contains(&settled.window) {
            windows.push(settled.window);
        }
    }
    windows.sort_by_key(Option::is_none);
    for window in windows {
        let round = held
            .iter()
            .filter(|settled| settled.window == window)
            .map(|settled| (settled.grant.clone(), settled.amount))
            .collect::<Vec<_>>();
        let left = request.remainder(&kept)?;
        let placed = assign(&round, &within_window(left, window)?)?;
        kept = kept.then(placed)?;
    }
    Ok(kept)
}

/// What a settlement of `window` may cover of `request`: its subscription's usage not dated before
/// the window, nothing else. Unchanged when the request names no usage of that subscription.
fn within_window(request: Request, window: Option<Window>) -> Result<Request, Overflow> {
    let Some((subscription, period_start)) = window else {
        return Ok(request);
    };
    let start = period_start.and_time(NaiveTime::MIN).and_utc();
    let of_subscription =
        |demand: &Demand| demand.usage && demand.subscription == Some(subscription);
    if !request.demands.iter().any(of_subscription) {
        return Ok(request);
    }
    let demands = request
        .demands
        .iter()
        .map(|demand| Demand {
            amount_minor: if of_subscription(demand)
                && demand.dated_before.is_none_or(|before| before > start)
            {
                demand.amount_minor
            } else {
                0
            },
            ..*demand
        })
        .collect::<Vec<_>>();
    let open = demands
        .iter()
        .try_fold(0_i64, |sum, demand| {
            sum.checked_add(demand.amount_minor.max(0))
        })
        .ok_or(Overflow)?;
    Ok(Request {
        requested_minor: request.requested_minor.max(0).min(open),
        demands,
    })
}

/// What promotional credit may take of `request` once the paid settlements the invoice keeps are
/// placed: each pays its charges tax included, and the usage that pays is never paid again
/// before tax.
pub fn after_settlements(
    request: &Request,
    charges: &[UsageCharge],
    paid_settled: &[Settled],
) -> Result<Request, SettleError> {
    if paid_settled.is_empty() || charges.is_empty() {
        return Ok(request.clone());
    }
    let owed = charges
        .iter()
        .map(|charge| charge.owed.clone())
        .collect::<Vec<_>>();
    let invoiced = UsageOwed::invoiced_together(
        &owed,
        &charges
            .iter()
            .map(|charge| charge.owed.base_minor)
            .collect::<Vec<_>>(),
    );
    let placed = assign_settled(
        paid_settled,
        &Request {
            requested_minor: invoiced
                .iter()
                .try_fold(0_i64, |sum, amount| sum.checked_add(*amount))
                .ok_or(Overflow)?,
            demands: charges
                .iter()
                .zip(&invoiced)
                .map(|(charge, amount)| Demand {
                    subscription: request
                        .demands
                        .get(charge.demand)
                        .and_then(|demand| demand.subscription),
                    product: charge.owed.product,
                    usage: true,
                    amount_minor: *amount,
                    dated_before: request
                        .demands
                        .get(charge.demand)
                        .and_then(|demand| demand.dated_before),
                })
                .collect(),
        },
    )?;
    let (mut base, mut paid) = (
        vec![0_i64; request.demands.len()],
        vec![0_i64; request.demands.len()],
    );
    for ((charge, covered), full) in charges.iter().zip(&placed.covered).zip(&invoiced) {
        let (Some(base), Some(paid)) = (base.get_mut(charge.demand), paid.get_mut(charge.demand))
        else {
            return Err(SettleError::UnknownDemand);
        };
        *base = base
            .checked_add(charge.owed.base_minor.max(0))
            .ok_or(Overflow)?;
        *paid = paid
            .checked_add(
                charge
                    .owed
                    .usage_paid_by_within(charge.owed.base_minor, *covered, *full),
            )
            .ok_or(Overflow)?;
    }
    let mut left = request.clone();
    let mut taken = 0_i64;
    for ((demand, base), paid) in left.demands.iter_mut().zip(base).zip(paid) {
        if paid <= 0 {
            continue;
        }
        let amount = demand.amount_minor.min(base - paid).max(0);
        taken = taken
            .checked_add(demand.amount_minor - amount)
            .ok_or(Overflow)?;
        demand.amount_minor = amount;
    }
    left.requested_minor = (left.requested_minor - taken).max(0);
    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credit::allocation::{Application, Classification, Scope};
    use chrono::{DateTime, TimeZone, Utc};
    use rust_decimal::Decimal;

    fn vat(base_minor: i64) -> UsageOwed {
        UsageOwed {
            product: None,
            base_minor,
            tax_inclusive: false,
            taxes: vec![Rate {
                rate: Decimal::new(20, 2),
                name: "VAT".to_owned(),
            }],
        }
    }

    fn customer_wide(at: DateTime<Utc>, classification: Classification) -> Grant {
        Grant {
            id: Uuid::from_u128(7),
            scope: Scope {
                subscription: None,
                application: Application::Invoice,
                products: Vec::new(),
            },
            remaining_minor: 0,
            priority: 100,
            classification,
            effective_at: at,
            pays_usage_from: at,
            lapses_at: None,
            created_at: at,
        }
    }

    /// 0.07 and 0.06 at 20%: their invoice rounds 20% once, so they owe 0.09 and 0.07 (OQ14).
    #[test]
    fn a_settlement_owes_each_product_its_share_of_the_tax_rounded_per_rate() {
        let owed = [vat(7), vat(6)];
        assert_eq!(owed[0].invoiced(7) + owed[1].invoiced(6), 15);
        let full = UsageOwed::invoiced_together(&owed, &[7, 6]);
        assert_eq!(full, vec![9, 7]);
        assert_eq!(owed[0].tax_in_within(7, 9, full[0]), 2);
        assert_eq!(owed[0].usage_paid_by_within(7, 8, full[0]), 6);
    }

    #[test]
    fn paid_credit_pays_the_most_usage_its_invoiced_figure_covers() {
        let owed = vat(5_000);
        assert_eq!(owed.usage_paid_by(10, 3), 2);
        assert_eq!(owed.usage_paid_by(5_000, 3_003), 2_502);
        assert_eq!(owed.usage_paid_by(5_000, 6_000), 5_000);
        assert_eq!(owed.usage_paid_by(5_000, 0), 0);
    }

    /// A settlement of the period starting Feb 17 covers that period's usage, never the window
    /// before it; with no window, grant order places it.
    #[test]
    fn settled_credit_covers_the_window_it_paid_never_an_earlier_one() {
        let day = |d| Utc.with_ymd_and_hms(2026, 2, d, 0, 0, 0).unwrap();
        let subscription = Uuid::from_u128(1);
        let usage = |amount_minor, before| Demand {
            subscription: Some(subscription),
            product: None,
            usage: true,
            amount_minor,
            dated_before: Some(before),
        };
        let request = Request {
            requested_minor: 75_258,
            demands: vec![
                usage(42_200, day(17)),
                usage(33_058, Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap()),
            ],
        };
        let settled = |window| Settled {
            window,
            grant: customer_wide(day(5), Classification::Paid),
            amount: 11_629,
        };
        let window = Some((subscription, NaiveDate::from_ymd_opt(2026, 2, 17).unwrap()));
        let kept = assign_settled(&[settled(window)], &request).unwrap();
        assert_eq!(kept.covered, vec![0, 11_629]);
        let unplaced = assign_settled(&[settled(None)], &request).unwrap();
        assert_eq!(unplaced.covered, vec![11_629, 0]);
    }

    /// 30.03 of paid credit settled on 50.00 of usage at 20% pays 25.02 of it: promotional credit
    /// may take the other 24.98, never placed on fixed fees.
    #[test]
    fn promotional_credit_takes_the_usage_paid_settlements_leave_before_tax() {
        let at = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let request = |amount_minor: i64| Request {
            requested_minor: amount_minor + 1_000,
            demands: vec![
                Demand {
                    subscription: None,
                    product: None,
                    usage: false,
                    amount_minor: 1_000,
                    dated_before: None,
                },
                Demand {
                    subscription: None,
                    product: None,
                    usage: true,
                    amount_minor,
                    dated_before: None,
                },
            ],
        };
        let charges = [UsageCharge {
            demand: 1,
            owed: vat(5_000),
        }];
        let paid = [Settled {
            window: None,
            grant: customer_wide(at, Classification::Paid),
            amount: 3_003,
        }];
        let left = after_settlements(&request(5_000), &charges, &paid).unwrap();
        assert_eq!(left.requested_minor, 3_498);
        assert_eq!(left.demands[0].amount_minor, 1_000);
        assert_eq!(left.demands[1].amount_minor, 2_498);
        let capped = after_settlements(&request(2_000), &charges, &paid).unwrap();
        assert_eq!(capped.demands[1].amount_minor, 2_000);
        assert_eq!(capped.requested_minor, 3_000);
    }
}
