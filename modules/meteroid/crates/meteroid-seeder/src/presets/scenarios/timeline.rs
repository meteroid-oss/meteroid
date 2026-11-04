//! A credit customer's history as data: what happened, in time order, and what the books must
//! show at the seed's `as_of`. The replay walks `History::events` in order, calling the engine at
//! each event's time; before applying an event at `t` it sweeps credit that expired before `t`.
//!
//! Times are relative to `as_of`, so the story reads the same whatever day the sandbox is created:
//! the open billing cycle has been running `cycle_age_days`, which fixes the billing day, and
//! earlier cycles fall on that day of earlier months.

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime};
use meteroid_store::domain::price_components::CreditExpiry;
use rust_decimal::Decimal;

use super::domain::{AutoTopUpSpec, BudgetSpec, CardSpec, EntitlementOverrideSpec};

#[derive(Clone, Debug, PartialEq)]
pub struct History {
    pub cycle_age_days: u32,
    /// Sales tax on every invoice of the customer; `InvoiceExpectation::tax` follows from it.
    pub sales_tax: Option<SalesTax>,
    pub events: Vec<Event>,
    pub usage: UsagePlan,
    pub expected: Expected,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SalesTax {
    pub name: &'static str,
    pub rate: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub at: When,
    pub action: Action,
}

impl Event {
    pub fn new(at: When, action: Action) -> Self {
        Event { at, action }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// `offset` after the start of the billing cycle `back` cycles before the one open at
    /// `as_of` (0 is that one); a negative `back` is a cycle still to come.
    Cycle {
        back: i32,
        offset: Duration,
    },
    Ago(Duration),
}

/// The start of billing cycle `back`.
pub fn cycle(back: i32) -> When {
    When::Cycle {
        back,
        offset: Duration::zero(),
    }
}

/// `days` days and `hour:minute` into billing cycle `back`; negative days fall before it.
pub fn cycle_at(back: i32, days: i64, hour: i64, minute: i64) -> When {
    When::Cycle {
        back,
        offset: Duration::days(days) + Duration::hours(hour) + Duration::minutes(minute),
    }
}

pub fn ago(duration: Duration) -> When {
    When::Ago(duration)
}

impl When {
    /// The same moment shifted by `minutes`, to order events that happen together.
    pub fn plus_minutes(self, minutes: i64) -> When {
        let delta = Duration::minutes(minutes);
        match self {
            When::Cycle { back, offset } => When::Cycle {
                back,
                offset: offset + delta,
            },
            When::Ago(d) => When::Ago(d - delta),
        }
    }
}

/// Resolves [`When`] for one customer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub as_of: NaiveDateTime,
    /// Day of month every billing cycle starts on, clamped to the month's length.
    pub billing_day: u32,
    current_start: NaiveDate,
}

impl Clock {
    pub fn new(as_of: NaiveDateTime, cycle_age_days: u32) -> Self {
        let current_start = as_of.date() - Duration::days(i64::from(cycle_age_days));
        Clock {
            as_of,
            billing_day: current_start.day(),
            current_start,
        }
    }

    pub fn cycle_start(&self, back: i32) -> NaiveDate {
        let first = self
            .current_start
            .with_day(1)
            .expect("the first of a month");
        let month = if back >= 0 {
            first - Months::new(back.unsigned_abs())
        } else {
            first + Months::new(back.unsigned_abs())
        };
        let last_day = (month + Months::new(1) - Duration::days(1)).day();
        month
            .with_day(self.billing_day.min(last_day))
            .expect("a day within the month")
    }

    pub fn resolve(&self, when: When) -> NaiveDateTime {
        match when {
            When::Cycle { back, offset } => {
                self.cycle_start(back).and_time(NaiveTime::MIN) + offset
            }
            When::Ago(d) => self.as_of - d,
        }
    }
}

/// A payment method on the mock gateway, named by a handle local to the history.
pub type CardHandle = &'static str;
/// An invoice or a credit purchase, named by a handle local to the history (numbers are the
/// engine's).
pub type Payable = &'static str;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Subscribe {
        plan: &'static str,
        /// End of a fixed contract term, the last day it covers.
        ends: Option<When>,
        /// Filled from the clock by `resolve_events`; passed to the engine explicitly, since the
        /// start date alone loses a clamped billing day (the 31st started on the 30th).
        billing_day_anchor: Option<u16>,
    },
    AddCard {
        card: CardHandle,
        spec: CardSpec,
        make_default: bool,
    },
    /// The card no longer exists on the customer (an updated card replaced it).
    RemoveCard {
        card: CardHandle,
    },
    SetDefaultCard {
        card: CardHandle,
    },
    /// Issues the subscription invoice of the billing cycle starting now: in-advance fees for the
    /// new cycle, usage of the one that ended. Credit on account is applied as it is issued.
    BillCycle {
        invoice: Payable,
        expected: InvoiceExpectation,
    },
    /// Issues a credit purchase invoice; the credit is funded once it is paid.
    Purchase {
        purchase: Payable,
        /// What the credit is granted in: the customer's currency, or a credit unit sold for it.
        denomination: &'static str,
        amount: Decimal,
        /// Months the credit stays spendable after payment.
        expires_in_months: Option<u32>,
    },
    /// The automatic top-up rule fires: a purchase charged to the default card at once.
    AutoTopUp {
        purchase: Payable,
        denomination: &'static str,
        amount: Decimal,
        expires_in_months: Option<u32>,
        outcome: ChargeOutcome,
    },
    /// Charges the default card for what is due on `payable`.
    Charge {
        payable: Payable,
        outcome: ChargeOutcome,
    },
    /// The customer's bank transfer for what is due on `payable` is received.
    BankTransfer {
        payable: Payable,
    },
    /// Redeems one of the scenario's promotion codes.
    RedeemPromoCode {
        code: &'static str,
    },
    /// Credit added by hand, outside any invoice.
    AddCredit {
        denomination: &'static str,
        amount: Decimal,
        reason: &'static str,
        promotional: bool,
        expires: Option<When>,
    },
    ConfigureWallet {
        denomination: &'static str,
        low_balance_thresholds: Vec<Decimal>,
        usage_at_zero: meteroid_store::domain::credit_exhausted::UsageAtZero,
        auto_top_up: Option<AutoTopUpSpec>,
    },
    SetEntitlementOverride(EntitlementOverrideSpec),
    SetBudget(BudgetSpec),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeOutcome {
    Paid,
    Declined(DeclineReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclineReason {
    CardExpired,
    /// Lost card: never retried.
    HardDecline,
}

/// What an issued subscription invoice must carry, before tax.
#[derive(Clone, Debug, PartialEq)]
pub struct InvoiceExpectation {
    /// Fees billed in advance for the new cycle, in the invoice currency.
    pub advance: Decimal,
    /// Usage of the cycle that ended, at list price in its pricing denomination.
    pub usage: Decimal,
    /// Usage left after credit in a credit unit, billed as overage in the invoice currency.
    pub overage: Decimal,
    pub minimum_shortfall: Decimal,
    /// Credit applied as the invoice is issued, by grant, in the grant's denomination.
    pub applied: Vec<(GrantRef, Decimal)>,
    /// Left to collect once credit is applied, before tax.
    pub amount_due: Decimal,
    /// Sales tax collected on top of `amount_due`, rounded once over the invoice.
    pub tax: Decimal,
}

impl InvoiceExpectation {
    pub fn empty() -> Self {
        InvoiceExpectation {
            advance: Decimal::ZERO,
            usage: Decimal::ZERO,
            overage: Decimal::ZERO,
            minimum_shortfall: Decimal::ZERO,
            applied: vec![],
            amount_due: Decimal::ZERO,
            tax: Decimal::ZERO,
        }
    }
}

/// Where a credit grant came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantRef {
    /// Credit a plan component granted on a paid invoice.
    PlanCredit {
        invoice: Payable,
        component: &'static str,
    },
    /// The bonus credit a plan pack sold below its face granted beside its paid credit.
    PlanBonus {
        invoice: Payable,
        component: &'static str,
    },
    /// What was left of a plan credit grant, carried past its expiry.
    Rollover {
        invoice: Payable,
        component: &'static str,
    },
    Purchase(Payable),
    PromoCode(&'static str),
    Added(&'static str),
}

/// How the customer's metered usage is generated: token mix, API keys and exact targets.
#[derive(Clone, Debug, PartialEq)]
pub struct UsagePlan {
    /// Share of tokens per model, in the order of `credit_traffic::MODELS`.
    pub mix: [u32; 3],
    pub api_keys: &'static [(&'static str, u32)],
    /// Intensity multiplier per 30 days inside a window, so usage ramps rather than steps.
    pub monthly_growth: f64,
    /// Requests per hour at peak, for the per-request events of the last 48 hours.
    pub runs_per_hour: f64,
    pub targets: Vec<UsageTarget>,
}

/// Usage in `[from, to)` hitting `amount` exactly. A target never spans a billing cycle boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageTarget {
    pub from: When,
    pub to: When,
    pub amount: UsageAmount,
    /// Cumulative priced usage reached at a time inside the window.
    pub checkpoints: Vec<(When, Decimal)>,
    pub spikes: Vec<Spike>,
}

impl UsageTarget {
    pub fn new(from: When, to: When, amount: UsageAmount) -> Self {
        UsageTarget {
            from,
            to,
            amount,
            checkpoints: vec![],
            spikes: vec![],
        }
    }

    pub fn with_checkpoint(mut self, at: When, cumulative: Decimal) -> Self {
        self.checkpoints.push((at, cumulative));
        self
    }

    pub fn with_spike(mut self, spike: Spike) -> Self {
        self.spikes.push(spike);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UsageAmount {
    /// Priced at the plan's list prices, in its pricing denomination, split by the token mix.
    Priced(Decimal),
    Tokens([u64; 3]),
    /// Tokens per API key and model.
    TokensByKey(Vec<(&'static str, [u64; 3])>),
}

/// A burst holding `amount` of priced usage in `[from, to)`, on one API key, on top of the rest.
#[derive(Clone, Debug, PartialEq)]
pub struct Spike {
    pub from: When,
    pub to: When,
    pub amount: Decimal,
    pub api_key: &'static str,
}

/// The books at `as_of`, counting usage before `as_of` only.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Expected {
    pub wallets: Vec<ExpectedWallet>,
    pub auto_top_up_suspended: bool,
    /// When usage stopped because the balance ran out.
    pub usage_paused_at: Option<When>,
    /// When the balance fell to the low-balance threshold.
    pub low_balance_at: Option<When>,
    /// When the plan's usage limit was reached.
    pub limit_reached_at: Option<When>,
    /// Issued invoices still awaiting payment, with their due date.
    pub open_invoices: Vec<(Payable, When)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExpectedWallet {
    pub denomination: &'static str,
    /// Credit on account: what is left of every live grant.
    pub on_account: Decimal,
    pub unbilled: Decimal,
    /// Credit on account less unbilled usage.
    pub balance: Decimal,
    /// What is left of each live grant, with when it expires.
    pub grants: Vec<ExpectedGrant>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExpectedGrant {
    pub source: GrantRef,
    pub remaining: Decimal,
    pub expires: Option<Expiry>,
}

/// When a grant expires, resolved with the engine's own rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    At(When),
    /// `expiry` after `from` (a payment, a redemption or an earlier expiry).
    After {
        from: When,
        expiry: CreditExpiry,
    },
}

impl Expiry {
    pub fn resolve(&self, clock: &Clock) -> Option<NaiveDateTime> {
        match self {
            Expiry::At(when) => Some(clock.resolve(*when)),
            Expiry::After { from, expiry } => expiry
                .resolve(clock.resolve(*from).and_utc())
                .map(|at| at.naive_utc()),
        }
    }
}

/// Every event of `history` at its time against `clock`, in order, with the billing day set.
pub fn resolve_events(history: &History, clock: &Clock) -> Vec<(NaiveDateTime, Action)> {
    history
        .events
        .iter()
        .map(|event| {
            let action = match event.action.clone() {
                Action::Subscribe { plan, ends, .. } => Action::Subscribe {
                    plan,
                    ends,
                    billing_day_anchor: Some(clock.billing_day as u16),
                },
                other => other,
            };
            (clock.resolve(event.at), action)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::utils::periods::find_period_containing_date;

    fn at(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    #[test]
    fn cycles_fall_on_the_billing_day_of_earlier_months() {
        let clock = Clock::new(at(2026, 10, 8, 19), 9);
        assert_eq!(clock.billing_day, 29);
        assert_eq!(
            clock.cycle_start(0),
            NaiveDate::from_ymd_opt(2026, 9, 29).unwrap()
        );
        assert_eq!(
            clock.cycle_start(3),
            NaiveDate::from_ymd_opt(2026, 6, 29).unwrap()
        );
        assert_eq!(
            clock.cycle_start(8),
            NaiveDate::from_ymd_opt(2026, 1, 29).unwrap()
        );
        assert_eq!(
            clock.cycle_start(7),
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        );
        assert_eq!(
            clock.resolve(cycle_at(1, 2, 10, 30)),
            at(2026, 8, 31, 10) + Duration::minutes(30)
        );
        assert_eq!(clock.resolve(ago(Duration::hours(5))), at(2026, 10, 8, 14));
    }

    #[test]
    fn subscriptions_carry_the_billing_day_the_start_date_loses() {
        // Billing on the 31st, started in a 30-day month.
        let clock = Clock::new(at(2026, 11, 7, 9), 7);
        assert_eq!(clock.billing_day, 31);
        let start = clock.cycle_start(1);
        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 9, 30).unwrap());

        // Without an explicit anchor the engine takes the start date's day.
        let derived = start.day();
        assert_eq!(derived, 30);
        let drifted = find_period_containing_date(
            start,
            clock.cycle_start(0),
            &BillingPeriodEnum::Monthly,
            derived,
        );
        assert_ne!(drifted.start, clock.cycle_start(0));

        let history = History {
            cycle_age_days: 7,
            sales_tax: None,
            events: vec![Event::new(
                cycle(1),
                Action::Subscribe {
                    plan: "Plan",
                    ends: None,
                    billing_day_anchor: None,
                },
            )],
            usage: UsagePlan {
                mix: [1, 0, 0],
                api_keys: &[("key", 1)],
                monthly_growth: 1.0,
                runs_per_hour: 1.0,
                targets: vec![],
            },
            expected: Expected::default(),
        };
        let resolved = resolve_events(&history, &clock);
        let Action::Subscribe {
            billing_day_anchor: Some(anchor),
            ..
        } = resolved[0].1
        else {
            panic!("a subscription with its billing day");
        };
        assert_eq!(u32::from(anchor), clock.billing_day);
        let period = find_period_containing_date(
            start,
            clock.cycle_start(0),
            &BillingPeriodEnum::Monthly,
            u32::from(anchor),
        );
        assert_eq!(period.start, clock.cycle_start(0));
    }

    #[test]
    fn cycle_starts_match_the_engine_periods() {
        let mut as_of = at(2026, 1, 1, 9);
        for _ in 0..800 {
            for age in [0u32, 5, 9, 21, 27, 30] {
                let clock = Clock::new(as_of, age);
                let start = clock.cycle_start(6);
                for back in 0..6 {
                    let begins = clock.cycle_start(back);
                    let period = find_period_containing_date(
                        start,
                        begins,
                        &BillingPeriodEnum::Monthly,
                        clock.billing_day,
                    );
                    assert_eq!(period.start, begins, "{as_of} age {age} back {back}");
                    assert_eq!(period.end, clock.cycle_start(back - 1));
                }
            }
            as_of += Duration::days(1);
        }
    }
}
