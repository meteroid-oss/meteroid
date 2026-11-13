//! The invoice reference model, slice S1: one customer on one subscription to a generated plan
//! (a fixed fee in advance or in arrears, and a per-unit metered fee), coupons taken at the
//! start, exclusive taxes on the customer, in USD, JPY or BHD; plan changes and cancellations at
//! the period end. Each figure is computed from the billing rules and cites its rule ID;
//! nothing here is shared with the engine. No credit.
use chrono::{DateTime, Duration, Months, NaiveDate, NaiveTime, Utc};

pub type At = DateTime<Utc>;

/// Backdated documents are issued during their day; the next action starts after them.
pub const DOCUMENTS: Duration = Duration::hours(13);

/// The last period end renewed: business time stays in the past.
pub fn last_renewal() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 1).expect("a day")
}

pub fn midnight(day: NaiveDate) -> At {
    day.and_time(NaiveTime::MIN).and_utc()
}

/// The end of the `cycle`-th period of a subscription anchored on its start day (R-015).
pub fn period_end(start: NaiveDate, cycle: u32) -> NaiveDate {
    start
        .checked_add_months(Months::new(cycle + 1))
        .expect("a later month")
}

/// `num / den` rounded once, half away from zero (R-001).
pub fn div_round(num: i128, den: i128) -> i64 {
    let (q, r) = (num / den, num % den);
    let q = if 2 * r.abs() >= den {
        q + num.signum()
    } else {
        q
    };
    i64::try_from(q).expect("an amount")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Currency {
    Usd,
    Jpy,
    Bhd,
}

impl Currency {
    pub fn code(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Jpy => "JPY",
            Self::Bhd => "BHD",
        }
    }

    /// Minor units per major unit as a power of ten (ISO 4217).
    pub fn exponent(self) -> u32 {
        match self {
            Self::Usd => 2,
            Self::Jpy => 0,
            Self::Bhd => 3,
        }
    }

    pub fn format(self, minor: i64) -> String {
        let e = self.exponent();
        if e == 0 {
            return minor.to_string();
        }
        let scale = 10_i64.pow(e);
        format!(
            "{}{}.{:0width$}",
            if minor < 0 { "-" } else { "" },
            minor.abs() / scale,
            minor.abs() % scale,
            width = e as usize
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Billing {
    Advance,
    Arrears,
}

/// Rates per unit are in ten-thousandths of the minor unit, finer than the currency.
pub const RATE_SCALE: i64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanSpec {
    /// The fixed fee per period, in minor units.
    pub fee: i64,
    pub billing: Billing,
    /// The metered fee per unit, in `1 / RATE_SCALE` minor units.
    pub rate: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discount {
    Percent(i64),
    /// In minor units.
    Fixed(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CouponSpec {
    pub discount: Discount,
    /// How many invoices it discounts; `None` is every one.
    pub cycles: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaxRate {
    pub code: &'static str,
    pub name: &'static str,
    /// Parts per million: 8.875 % is 88_750.
    pub ppm: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    pub currency: Currency,
    /// Plan A, which the subscription starts on, and plan B.
    pub plans: [PlanSpec; 2],
    /// At most one percentage coupon and one fixed one, so their order is the rule's (R-020).
    pub coupons: Vec<CouponSpec>,
    pub taxes: Vec<TaxRate>,
}

pub fn plan_name(plan: usize) -> &'static str {
    ["A", "B"][plan]
}

pub fn fee_line(plan: usize) -> String {
    format!("{} fee", plan_name(plan))
}

pub fn units_line(plan: usize) -> String {
    format!("{} units", plan_name(plan))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheduled {
    Change(usize),
    Cancel,
}

#[derive(Clone, Debug)]
pub struct Sub {
    pub plan: usize,
    pub start: NaiveDate,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub cycle: u32,
    pub active: bool,
    pub scheduled: Option<Scheduled>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invoice {
    /// (name, subtotal) of every line billing something, sorted.
    pub lines: Vec<(String, i64)>,
    pub subtotal: i64,
    pub discount: i64,
    pub tax: i64,
    pub total: i64,
}

impl Invoice {
    /// Nothing is paid in S1: `due = max(0, total − applied − paid)` (R1).
    pub fn due(&self) -> i64 {
        self.total.max(0)
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Start {
        day: NaiveDate,
        catalog: Catalog,
    },
    Usage {
        at: At,
        units: i64,
    },
    /// The period end: a renewal, or the final invoice of a cancellation.
    Advance {
        at: At,
    },
    ChangeAtEnd {
        at: At,
        to: usize,
    },
    CancelAtEnd {
        at: At,
        end: NaiveDate,
    },
}

impl Action {
    pub fn at(&self) -> At {
        match self {
            Self::Start { day, .. } => midnight(*day),
            Self::Usage { at, .. }
            | Self::Advance { at }
            | Self::ChangeAtEnd { at, .. }
            | Self::CancelAtEnd { at, .. } => *at,
        }
    }

    /// The instant the action's last step happens, after which the next action may start.
    pub fn done(&self) -> At {
        match self {
            Self::Advance { at } => *at + DOCUMENTS,
            other => other.at(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub &'static str);

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub now: Option<At>,
    pub catalog: Option<Catalog>,
    pub sub: Option<Sub>,
    pub usage: Vec<(At, i64)>,
    pub invoices: Vec<Invoice>,
    /// Invoices each coupon discounted so far.
    pub uses: Vec<u32>,
}

impl Model {
    pub fn now(&self) -> At {
        self.now.expect("started")
    }

    pub fn catalog(&self) -> &Catalog {
        self.catalog.as_ref().expect("started")
    }

    pub fn sub(&self) -> &Sub {
        self.sub.as_ref().expect("started")
    }

    pub fn apply(&mut self, action: &Action) -> Result<(), Refused> {
        match action {
            Action::Start { day, catalog } => self.start(*day, catalog.clone()),
            Action::Usage { at, units } => self.usage.push((*at, *units)),
            Action::Advance { at } => self.advance(*at),
            Action::ChangeAtEnd { to, .. } => {
                self.sub.as_mut().expect("started").scheduled = Some(Scheduled::Change(*to));
            }
            Action::CancelAtEnd { .. } => {
                self.sub.as_mut().expect("started").scheduled = Some(Scheduled::Cancel);
            }
        }
        self.now = Some(action.done());
        Ok(())
    }

    fn start(&mut self, day: NaiveDate, catalog: Catalog) {
        self.uses = vec![0; catalog.coupons.len()];
        let plan = catalog.plans[0];
        self.catalog = Some(catalog);
        self.sub = Some(Sub {
            plan: 0,
            start: day,
            period_start: day,
            period_end: period_end(day, 0),
            cycle: 0,
            active: true,
            scheduled: None,
        });
        // R-011: the opening invoice bills the first period's advance and no arrears; R-002: the
        // period starts on its anchor day, so it is whole and nothing is prorated.
        let mut lines = vec![];
        if plan.billing == Billing::Advance {
            lines.push((fee_line(0), plan.fee));
        }
        self.issue(lines);
    }

    /// The period's usage, billed whole at its end: usage is never prorated (R-002); the line is
    /// units × rate rounded once to the minor unit (R-001).
    fn usage_line(&self, plan: usize, from: At, until: At) -> (String, i64) {
        let units: i64 = self
            .usage
            .iter()
            .filter(|(at, _)| from <= *at && *at < until)
            .map(|(_, units)| units)
            .sum();
        let rate = self.catalog().plans[plan].rate;
        (
            units_line(plan),
            div_round(i128::from(units) * i128::from(rate), i128::from(RATE_SCALE)),
        )
    }

    fn advance(&mut self, at: At) {
        let sub = self.sub().clone();
        let elapsed = self.catalog().plans[sub.plan];
        // R-011: the invoice at a period end bills the elapsed period's arrears, the fixed fee
        // whole as the period ran to its end (R-002) and its usage.
        let mut lines = vec![self.usage_line(sub.plan, midnight(sub.period_start), at)];
        if elapsed.billing == Billing::Arrears {
            lines.push((fee_line(sub.plan), elapsed.fee));
        }
        let next = match sub.scheduled {
            // R-016: a cancellation effective at the period end settles at its midnight, on a
            // final invoice with no new period.
            Some(Scheduled::Cancel) => None,
            Some(Scheduled::Change(to)) => Some(to),
            None => Some(sub.plan),
        };
        if let Some(plan) = next {
            // R-011: and the new period's advance, on the plan in force from then.
            let renewed = self.catalog().plans[plan];
            if renewed.billing == Billing::Advance {
                lines.push((fee_line(plan), renewed.fee));
            }
        }
        let s = self.sub.as_mut().expect("started");
        s.scheduled = None;
        match next {
            None => s.active = false,
            Some(plan) => {
                s.plan = plan;
                s.cycle += 1;
                s.period_start = s.period_end;
                s.period_end = period_end(s.start, s.cycle);
            }
        }
        self.issue(lines);
    }

    /// An invoice of `lines`; one that bills nothing is no document.
    fn issue(&mut self, lines: Vec<(String, i64)>) {
        let mut lines: Vec<(String, i64)> = lines.into_iter().filter(|(_, a)| *a != 0).collect();
        if lines.is_empty() {
            return;
        }
        lines.sort();
        let subtotal = lines.iter().map(|(_, a)| a).sum();
        let discount = self.discount(subtotal);
        // R-021: coupons come off before tax. R-030: each rate's tax is rounded once over the
        // invoice (R-001).
        let taxable = subtotal - discount;
        let tax = self
            .catalog()
            .taxes
            .iter()
            .map(|t| div_round(i128::from(taxable) * i128::from(t.ppm), 1_000_000))
            .sum::<i64>();
        self.invoices.push(Invoice {
            lines,
            subtotal,
            discount,
            tax,
            // I2 / R1: total = subtotal − discount − promotional + tax, with no credit here.
            total: subtotal - discount + tax,
        });
    }

    /// R-020: percentage coupons first, then fixed ones, each on what the previous left, never
    /// above the subtotal; a coupon discounts as many invoices as its duration.
    fn discount(&mut self, subtotal: i64) -> i64 {
        let mut order: Vec<usize> = (0..self.uses.len())
            .filter(|&c| {
                self.catalog().coupons[c]
                    .cycles
                    .is_none_or(|n| self.uses[c] < n)
            })
            .collect();
        order.sort_by_key(|&c| matches!(self.catalog().coupons[c].discount, Discount::Fixed(_)));
        // What is left to discount, exact, in hundredths of the minor unit.
        let mut left = i128::from(subtotal) * 100;
        let mut discount = 0;
        for c in order {
            // KW-09: coupons stop once at most one minor unit is left.
            if left <= 100 {
                break;
            }
            let off = match self.catalog().coupons[c].discount {
                Discount::Percent(p) => left * i128::from(p) / 100,
                Discount::Fixed(amount) => (i128::from(amount) * 100).min(left),
            };
            left -= off;
            // KW-01: each coupon's discount is truncated to the minor unit, not rounded (R-001),
            // and the fraction stays in what the next coupon applies to.
            discount += i64::try_from(off / 100).expect("an amount");
            self.uses[c] += 1;
        }
        discount
    }
}
