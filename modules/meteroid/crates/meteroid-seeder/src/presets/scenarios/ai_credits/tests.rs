//! Checks every Halyard history on paper: events in time order, usage inside its cycles, and the
//! expected books reproduced by a plain simulation of the credit rules (credit applies by soonest
//! expiry, promotional before paid, then oldest; currency credit pays everything but credit
//! purchases; credit expiring at `t` still pays what is billed at `t`).

use std::collections::HashMap;

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use meteroid_store::domain::BillingPeriodEnum;
use meteroid_store::domain::price_components::{CreditExpiry, CreditRollover};
use rust_decimal::{Decimal, RoundingStrategy};
use rust_decimal_macros::dec;

use super::*;
use crate::presets::credit_traffic::TRAFFIC;
use crate::presets::credits::model_prices;
use crate::presets::history_usage::{self, ModelPrices};
use crate::presets::scenarios::domain::{
    AutoTopUpAmount, AutoTopUpSpec, CardBehavior, CardSpec, Customer, FeeType, Plan,
    PlanEntitlementSpec, Scenario,
};
use crate::presets::scenarios::timeline::{
    Action, ChargeOutcome, Clock, DeclineReason, GrantRef, History, InvoiceExpectation,
    UsageAmount, When, cycle, cycle_at, resolve_events,
};
use crate::utils::slugify;
use common_domain::ids::CustomerId;

fn reference_as_of() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 8)
        .unwrap()
        .and_hms_opt(19, 13, 0)
        .unwrap()
}

fn histories(scenario: &Scenario) -> Vec<(&Customer, &History)> {
    scenario
        .customers
        .iter()
        .map(|c| {
            (
                c,
                c.history
                    .as_ref()
                    .expect("every Halyard account has a history"),
            )
        })
        .collect()
}

fn plan_of<'a>(scenario: &'a Scenario, history: &History) -> &'a Plan {
    let name = history
        .events
        .iter()
        .find_map(|e| match &e.action {
            Action::Subscribe { plan, .. } => Some(*plan),
            _ => None,
        })
        .expect("a subscription");
    scenario.plans.iter().find(|p| p.name == name).unwrap()
}

fn prices_of(scenario: &Scenario, history: &History) -> Option<ModelPrices> {
    model_prices(plan_of(scenario, history), &scenario.credit_units)
}

#[test]
fn the_lineup_prices_models_per_million_tokens() {
    let scenario = ai_credits_scenario();
    let plan = |name: &str| scenario.plans.iter().find(|p| p.name == name).unwrap();
    let list = model_prices(plan(PAY_AS_YOU_GO), &scenario.credit_units).unwrap();
    assert_eq!(list.per_million, [dec!(2), dec!(12), dec!(60)]);
    assert_eq!(
        model_prices(plan(SCALE), &scenario.credit_units),
        Some(list.clone())
    );
    let enterprise = model_prices(plan(ENTERPRISE), &scenario.credit_units).unwrap();
    for (e, l) in enterprise.per_million.iter().zip(list.per_million) {
        assert_eq!(*e, l * dec!(0.8));
    }
    let studio = model_prices(plan(STUDIO), &scenario.credit_units).unwrap();
    assert_eq!(studio.scale, 0);
    for (credits, l) in studio.per_million.iter().zip(list.per_million) {
        assert_eq!(*credits * CREDIT_VALUE, l);
    }
    assert!(model_prices(plan(FREE), &scenario.credit_units).is_none());

    // One limit per plan: the credit balance on credit plans, a token cap only on Free.
    for plan in &scenario.plans {
        let expected = if plan.name == FREE { 2 } else { 0 };
        assert_eq!(plan.entitlements.len(), expected, "{}", plan.name);
    }
    assert!(matches!(
        plan(FREE).entitlement(TOKENS_FEATURE),
        Some(PlanEntitlementSpec::Metered { block: Some(_), .. })
    ));
    assert_eq!(plan(ENTERPRISE).minimum, Some(dec!(5000)));
    assert_eq!(plan(ENTERPRISE).payment.net_terms, 30);
    assert_eq!(plan(SCALE).payment.net_terms, 0);
}

#[test]
fn every_account_has_a_traffic_profile() {
    let scenario = ai_credits_scenario();
    assert_eq!(scenario.customers.len(), 8);
    for customer in &scenario.customers {
        assert!(
            traffic_for(&slugify(&customer.name)).is_some(),
            "{}",
            customer.name
        );
    }
    assert_eq!(TRAFFIC.len(), scenario.customers.len());
}

#[test]
fn events_are_time_ordered_and_stay_in_the_past_whatever_the_seed_day() {
    let scenario = ai_credits_scenario();
    let mut as_of = NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_time(NaiveTime::from_hms_opt(0, 30, 0).unwrap());
    for _ in 0..800 {
        for (customer, history) in histories(&scenario) {
            let clock = Clock::new(as_of, history.cycle_age_days);
            let times: Vec<NaiveDateTime> =
                history.events.iter().map(|e| clock.resolve(e.at)).collect();
            assert!(
                times.windows(2).all(|w| w[0] <= w[1]),
                "{} events out of order at {as_of}",
                customer.name
            );
            assert!(
                times.iter().all(|t| *t < as_of),
                "{} at {as_of}",
                customer.name
            );
            for target in &history.usage.targets {
                let (from, to) = (clock.resolve(target.from), clock.resolve(target.to));
                assert!(
                    from < to && to <= as_of,
                    "{} window at {as_of}",
                    customer.name
                );
                let back = (0..12)
                    .find(|k| clock.cycle_start(*k).and_time(NaiveTime::MIN) <= from)
                    .unwrap();
                let end = clock.cycle_start(back - 1).and_time(NaiveTime::MIN);
                assert!(
                    to <= end,
                    "{} window spans a cycle at {as_of}",
                    customer.name
                );
                for (at, _) in &target.checkpoints {
                    let at = clock.resolve(*at);
                    assert!(
                        from < at && at < to,
                        "{} checkpoint at {as_of}",
                        customer.name
                    );
                }
            }
        }
        as_of += Duration::hours(13);
    }
}

#[test]
fn the_books_follow_from_the_histories() {
    let scenario = ai_credits_scenario();
    for days in 0..400 {
        let as_of = reference_as_of() + Duration::days(days);
        for (customer, history) in histories(&scenario) {
            Simulation::new(&scenario, customer, history, as_of).run();
        }
    }
}

#[test]
fn generated_usage_hits_every_target_and_stays_small() {
    let scenario = ai_credits_scenario();
    let as_of = reference_as_of();
    let mut events = 0;
    for (customer, history) in histories(&scenario) {
        let clock = Clock::new(as_of, history.cycle_age_days);
        let prices = prices_of(&scenario, history);
        let id = CustomerId::from_const(uuid::Uuid::from_u128(events as u128 + 1));
        let usage = history_usage::generate(id, &history.usage, &clock, prices.as_ref())
            .unwrap_or_else(|e| panic!("{}: {e}", customer.name));
        for (window, target) in usage.windows.iter().zip(&history.usage.targets) {
            match &target.amount {
                UsageAmount::Priced(amount) => {
                    assert_eq!(prices.as_ref().unwrap().price(window.tokens), *amount)
                }
                UsageAmount::Tokens(tokens) => assert_eq!(window.tokens, *tokens),
                UsageAmount::TokensByKey(by_key) => {
                    let mut sum = [0u64; 3];
                    for (_, t) in by_key {
                        for m in 0..3 {
                            sum[m] += t[m];
                        }
                    }
                    assert_eq!(window.tokens, sum);
                }
            }
        }
        let summed: u64 = usage
            .events
            .iter()
            .map(|e| e.properties[0].1.parse::<u64>().unwrap())
            .sum();
        assert_eq!(summed, usage.total_tokens(), "{}", customer.name);
        events += usage.events.len();
    }
    assert!(events < 16_000, "{events} events");
    assert!(events > 4_000, "{events} events");
}

#[test]
fn quillbase_runaway_batch_is_about_six_million_tokens() {
    let scenario = ai_credits_scenario();
    let quillbase = scenario
        .customers
        .iter()
        .find(|c| c.name == "Quillbase")
        .unwrap();
    let history = quillbase.history.as_ref().unwrap();
    let spike = &history.usage.targets[0].spikes[0];
    let prices = prices_of(&scenario, history).unwrap();
    let tokens =
        history_usage::solve(spike.amount, &prices.per_million, 2, history.usage.mix).unwrap();
    let total: u64 = tokens.iter().sum();
    assert!((5_500_000..=6_500_000).contains(&total), "{total}");
}

#[derive(Clone, Copy)]
enum Probe {
    LowBalance,
    Paused,
}

/// The engine's default grant priority; a rolled grant draws one step ahead of it.
const DEFAULT_PRIORITY: i32 = 100;

struct Grant {
    source: GrantRef,
    denomination: &'static str,
    remaining: Decimal,
    priority: i32,
    effective_at: NaiveDateTime,
    expires_at: Option<NaiveDateTime>,
    promotional: bool,
    rollover: Option<CreditRollover>,
    seq: usize,
}

impl Grant {
    /// Not spendable at its expiry instant, as the engine's allocation.
    fn spendable_at(&self, at: NaiveDateTime) -> bool {
        self.remaining > Decimal::ZERO
            && self.effective_at <= at
            && self.expires_at.is_none_or(|expires| expires > at)
    }
}

/// A paid pack, minted when its invoice is paid.
struct PaidPack {
    component: &'static str,
    amount: Decimal,
    price: Decimal,
    denomination: &'static str,
    expiry: CreditExpiry,
}

struct Invoice {
    issued_at: NaiveDateTime,
    period: (NaiveDateTime, NaiveDateTime),
    due: Decimal,
    paid: bool,
    packs: Vec<PaidPack>,
}

/// Usage an expiring grant paid before lapsing, taken over by the next invoice of the period.
struct Settlement {
    source: GrantRef,
    denomination: &'static str,
    amount: Decimal,
}

struct Window {
    from: NaiveDateTime,
    to: NaiveDateTime,
    total: Decimal,
    tokens: u64,
    checkpoints: Vec<(NaiveDateTime, Decimal)>,
}

impl Window {
    fn cumulative(&self, at: NaiveDateTime) -> Decimal {
        if at <= self.from {
            return Decimal::ZERO;
        }
        if at >= self.to {
            return self.total;
        }
        self.checkpoints
            .iter()
            .find(|(t, _)| *t == at)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("usage at {at} needs a checkpoint"))
    }
}

fn component_name(name: &str) -> &'static str {
    [MONTHLY_CREDIT_PACK, WELCOME_CREDIT, INCLUDED_CREDITS]
        .into_iter()
        .find(|n| *n == name)
        .unwrap_or_else(|| panic!("unknown credit component {name}"))
}

/// The engine's term for purchased credit: a period-bound pack still grants one whole period
/// from payment.
fn purchased_expiry(
    expiry: CreditExpiry,
    paid_at: NaiveDateTime,
    period: (NaiveDateTime, NaiveDateTime),
) -> Option<NaiveDateTime> {
    match expiry {
        CreditExpiry::BillingPeriod => Some(period.1.max(paid_at + (period.1 - period.0))),
        other => other.resolve(paid_at.and_utc()).map(|t| t.naive_utc()),
    }
}

struct Simulation<'a> {
    scenario: &'a Scenario,
    name: &'a str,
    history: &'a History,
    clock: Clock,
    plan: &'a Plan,
    pricing: &'static str,
    windows: Vec<Window>,
    grants: Vec<Grant>,
    seq: usize,
    invoices: Vec<(&'static str, Invoice)>,
    settlements: Vec<Settlement>,
    purchases: HashMap<&'static str, (&'static str, Decimal, Option<u32>)>,
    cards: HashMap<&'static str, CardSpec>,
    default_card: Option<&'static str>,
    auto_top_up: Option<AutoTopUpSpec>,
    low_balance: Vec<Decimal>,
    suspended: bool,
    last_bill: Option<NaiveDateTime>,
}

impl<'a> Simulation<'a> {
    fn new(
        scenario: &'a Scenario,
        customer: &'a Customer,
        history: &'a History,
        as_of: NaiveDateTime,
    ) -> Self {
        let clock = Clock::new(as_of, history.cycle_age_days);
        let plan = plan_of(scenario, history);
        let prices = model_prices(plan, &scenario.credit_units);
        let pricing = plan
            .components
            .iter()
            .find_map(|c| match &c.fee {
                FeeType::Usage {
                    credit_unit: Some(_),
                    ..
                } => Some(CREDITS),
                _ => None,
            })
            .unwrap_or(USD);
        let windows = history
            .usage
            .targets
            .iter()
            .map(|t| {
                let tokens = match &t.amount {
                    UsageAmount::Tokens(tokens) => *tokens,
                    UsageAmount::TokensByKey(by_key) => {
                        by_key.iter().fold([0; 3], |acc, (_, t)| {
                            [acc[0] + t[0], acc[1] + t[1], acc[2] + t[2]]
                        })
                    }
                    UsageAmount::Priced(_) => [0; 3],
                };
                let total = match &t.amount {
                    UsageAmount::Priced(amount) => *amount,
                    _ => prices.as_ref().map(|p| p.price(tokens)).unwrap_or_default(),
                };
                Window {
                    from: clock.resolve(t.from),
                    to: clock.resolve(t.to),
                    total,
                    tokens: tokens.iter().sum(),
                    checkpoints: t
                        .checkpoints
                        .iter()
                        .map(|(at, c)| (clock.resolve(*at), *c))
                        .collect(),
                }
            })
            .collect();
        Simulation {
            scenario,
            name: &customer.name,
            history,
            clock,
            plan,
            pricing,
            windows,
            grants: vec![],
            seq: 0,
            invoices: vec![],
            settlements: vec![],
            purchases: HashMap::new(),
            cards: HashMap::new(),
            default_card: None,
            auto_top_up: None,
            low_balance: vec![],
            suspended: false,
            last_bill: None,
        }
    }

    fn context(&self, at: NaiveDateTime) -> String {
        format!("{} at {at} (as of {})", self.name, self.clock.as_of)
    }

    fn usage_between(&self, from: NaiveDateTime, to: NaiveDateTime) -> Decimal {
        self.windows
            .iter()
            .map(|w| w.cumulative(to) - w.cumulative(from))
            .sum()
    }

    fn unbilled(&self, at: NaiveDateTime) -> Decimal {
        self.last_bill
            .map(|bill| self.usage_between(bill, at))
            .unwrap_or_default()
    }

    fn on_account(&self, denomination: &str) -> Decimal {
        self.grants
            .iter()
            .filter(|g| g.denomination == denomination)
            .map(|g| g.remaining)
            .sum()
    }

    fn settled(&self, denomination: &str) -> Decimal {
        self.settlements
            .iter()
            .filter(|s| s.denomination == denomination)
            .map(|s| s.amount)
            .sum()
    }

    /// Usage settled from expiring credit has already left the wallet.
    fn balance(&self, at: NaiveDateTime) -> Decimal {
        self.on_account(self.pricing) - (self.unbilled(at) - self.settled(self.pricing))
    }

    #[allow(clippy::too_many_arguments)]
    fn grant(
        &mut self,
        source: GrantRef,
        denomination: &'static str,
        amount: Decimal,
        priority: i32,
        effective_at: NaiveDateTime,
        expires_at: Option<NaiveDateTime>,
        promotional: bool,
        rollover: Option<CreditRollover>,
    ) {
        self.seq += 1;
        self.grants.push(Grant {
            source,
            denomination,
            remaining: amount,
            priority,
            effective_at,
            expires_at,
            promotional,
            rollover,
            seq: self.seq,
        });
    }

    /// Lapses what expired by `at`, earliest first: each grant first pays the usage it covered
    /// up to its expiry, then carries a capped leftover into a rolled grant that draws first.
    fn sweep(&mut self, at: NaiveDateTime) {
        while let Some(i) = (0..self.grants.len())
            .filter(|i| self.grants[*i].expires_at.is_some_and(|e| e <= at))
            .min_by_key(|i| (self.grants[*i].expires_at, self.draw_key(*i)))
        {
            let mut g = self.grants.remove(i);
            let expired_at = g.expires_at.expect("an expiring grant");
            if g.remaining > Decimal::ZERO
                && g.denomination == self.pricing
                && let Some(bill) = self.last_bill
            {
                let pending = self.usage_between(bill, expired_at) - self.settled(g.denomination);
                let take = pending.max(Decimal::ZERO).min(g.remaining);
                if take > Decimal::ZERO {
                    g.remaining -= take;
                    self.settlements.push(Settlement {
                        source: g.source,
                        denomination: g.denomination,
                        amount: take,
                    });
                }
            }
            let (Some(rollover), GrantRef::PlanCredit { invoice, component }) =
                (g.rollover, g.source)
            else {
                continue;
            };
            let carried = rollover
                .max_amount
                .map_or(g.remaining, |cap| g.remaining.min(cap));
            if carried.is_zero() {
                continue;
            }
            let expires = rollover
                .expiry
                .resolve(expired_at.and_utc())
                .map(|t| t.naive_utc());
            self.grant(
                GrantRef::Rollover { invoice, component },
                g.denomination,
                carried,
                g.priority - 1,
                expired_at,
                expires,
                g.promotional,
                None,
            );
        }
    }

    /// Lower priority first, then the soonest expiry, promotional before paid, then the oldest.
    fn draw_key(&self, i: usize) -> (i32, NaiveDateTime, bool, NaiveDateTime, usize) {
        let g = &self.grants[i];
        (
            g.priority,
            g.expires_at.unwrap_or(NaiveDateTime::MAX),
            !g.promotional,
            g.effective_at,
            g.seq,
        )
    }

    fn apply(
        &mut self,
        denomination: &str,
        amount: Decimal,
        at: NaiveDateTime,
    ) -> Vec<(GrantRef, Decimal)> {
        let mut order: Vec<usize> = (0..self.grants.len())
            .filter(|i| {
                self.grants[*i].denomination == denomination && self.grants[*i].spendable_at(at)
            })
            .collect();
        order.sort_by_key(|i| self.draw_key(*i));
        let mut left = amount;
        let mut applied = vec![];
        for i in order {
            if left.is_zero() {
                break;
            }
            let take = self.grants[i].remaining.min(left);
            self.grants[i].remaining -= take;
            left -= take;
            applied.push((self.grants[i].source, take));
        }
        applied
    }

    fn charge_outcome(&self) -> ChargeOutcome {
        let card = self
            .default_card
            .and_then(|c| self.cards.get(c))
            .expect("a default card");
        match card.behavior {
            CardBehavior::Approved => ChargeOutcome::Paid,
            CardBehavior::Expired => ChargeOutcome::Declined(DeclineReason::CardExpired),
            CardBehavior::HardDeclined => ChargeOutcome::Declined(DeclineReason::HardDecline),
            CardBehavior::Declined => panic!("no Halyard history uses a soft decline"),
        }
    }

    fn settle(&mut self, payable: &'static str, at: NaiveDateTime) {
        if let Some((_, invoice)) = self.invoices.iter_mut().find(|(h, _)| *h == payable) {
            assert!(!invoice.paid, "{payable} paid twice");
            invoice.paid = true;
            let period = invoice.period;
            let packs = std::mem::take(&mut invoice.packs);
            for pack in packs {
                let expires = purchased_expiry(pack.expiry, at, period);
                // Money credit sold below face is paid credit worth its price plus bonus credit.
                let paid = if pack.denomination == USD {
                    pack.amount.min(pack.price)
                } else {
                    pack.amount
                };
                let component = pack.component;
                self.grant(
                    GrantRef::PlanCredit {
                        invoice: payable,
                        component,
                    },
                    pack.denomination,
                    paid,
                    DEFAULT_PRIORITY,
                    at,
                    expires,
                    false,
                    None,
                );
                if paid < pack.amount {
                    self.grant(
                        GrantRef::PlanBonus {
                            invoice: payable,
                            component,
                        },
                        pack.denomination,
                        pack.amount - paid,
                        DEFAULT_PRIORITY,
                        at,
                        expires,
                        true,
                        None,
                    );
                }
            }
            return;
        }
        let (denomination, amount, months) = self
            .purchases
            .remove(payable)
            .unwrap_or_else(|| panic!("{payable} is unknown"));
        let expires = months.map(|m| {
            CreditExpiry::Month(m)
                .resolve(at.and_utc())
                .unwrap()
                .naive_utc()
        });
        self.grant(
            GrantRef::Purchase(payable),
            denomination,
            amount,
            DEFAULT_PRIORITY,
            at,
            expires,
            false,
            None,
        );
    }

    fn bill(&mut self, invoice: &'static str, at: NaiveDateTime, expected: &InvoiceExpectation) {
        let ctx = self.context(at);
        let first = self.last_bill.is_none();
        let usage = self.unbilled(at);
        let back = (0..24)
            .find(|k| self.clock.cycle_start(*k) == at.date())
            .unwrap_or_else(|| panic!("{invoice} is not issued on a billing day, {ctx}"));
        let period = (
            at,
            self.clock.cycle_start(back - 1).and_time(NaiveTime::MIN),
        );

        // Fixed fees and paid packs bill in advance; only fees are payable from currency credit.
        let mut fees = Decimal::ZERO;
        let mut packs_price = Decimal::ZERO;
        let mut paid_packs = vec![];
        let mut included = vec![];
        for component in &self.plan.components {
            match &component.fee {
                FeeType::Rate { rates } => {
                    fees += rates
                        .iter()
                        .find(|r| r.term == BillingPeriodEnum::Monthly)
                        .expect("a monthly rate")
                        .price;
                }
                FeeType::Credit {
                    unit_price,
                    quantity,
                    credit_amount,
                    expiry,
                    credit_unit,
                    rollover,
                    once,
                } => {
                    if *once && !first {
                        continue;
                    }
                    let qty = Decimal::from(*quantity);
                    let denomination = match credit_unit.as_deref() {
                        Some(CREDITS) => CREDITS,
                        Some(other) => panic!("unknown unit {other}"),
                        None => USD,
                    };
                    let expiry = expiry.expect("Halyard credit expires");
                    let component = component_name(&component.name);
                    if unit_price.is_zero() {
                        included.push((
                            component,
                            credit_amount * qty,
                            denomination,
                            expiry,
                            *rollover,
                        ));
                    } else {
                        packs_price += unit_price * qty;
                        paid_packs.push(PaidPack {
                            component,
                            amount: credit_amount * qty,
                            price: unit_price * qty,
                            denomination,
                            expiry,
                        });
                    }
                }
                _ => {}
            }
        }
        let advance = fees + packs_price;

        // The period's settlements are netted first, then spendable credit is drawn.
        let mut applied: Vec<(GrantRef, Decimal)> = self
            .settlements
            .iter()
            .map(|s| (s.source, s.amount))
            .collect();
        let settled = self.settled(self.pricing);
        let mut overage = Decimal::ZERO;
        let money_usage = if self.pricing == CREDITS {
            let drawn = self.apply(CREDITS, usage - settled, at);
            let covered: Decimal = drawn.iter().map(|(_, a)| *a).sum();
            applied.extend(drawn);
            overage = (usage - settled - covered) * CREDIT_VALUE;
            Decimal::ZERO
        } else {
            usage
        };
        let shortfall = match self.plan.minimum {
            Some(minimum) if !first => (minimum - money_usage).max(Decimal::ZERO),
            _ => Decimal::ZERO,
        };
        let money_settled = if self.pricing == USD {
            settled
        } else {
            Decimal::ZERO
        };
        let payable = fees + money_usage - money_settled + overage + shortfall;
        let money_applied = self.apply(USD, payable, at);
        let covered: Decimal = money_applied.iter().map(|(_, a)| *a).sum();
        let promotional_covered: Decimal = money_applied
            .iter()
            .filter(|(source, _)| {
                self.grants
                    .iter()
                    .any(|g| g.source == *source && g.promotional)
            })
            .map(|(_, a)| *a)
            .sum();
        applied.extend(money_applied);
        let due = packs_price + payable - covered;

        // Promotional credit comes off before tax; paid credit pays the taxed total.
        let tax = match self.history.sales_tax {
            Some(sales_tax) => {
                assert_eq!(
                    covered, promotional_covered,
                    "paid currency credit on a taxed invoice, {ctx}"
                );
                assert!(packs_price.is_zero(), "a taxed history buys no pack, {ctx}");
                (due * sales_tax.rate)
                    .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
            }
            None => Decimal::ZERO,
        };

        assert_eq!(advance, expected.advance, "advance of {invoice}, {ctx}");
        assert_eq!(usage, expected.usage, "usage of {invoice}, {ctx}");
        assert_eq!(overage, expected.overage, "overage of {invoice}, {ctx}");
        assert_eq!(
            shortfall, expected.minimum_shortfall,
            "shortfall of {invoice}, {ctx}"
        );
        assert_eq!(
            applied, expected.applied,
            "credit applied to {invoice}, {ctx}"
        );
        assert_eq!(due, expected.amount_due, "due on {invoice}, {ctx}");
        assert_eq!(tax, expected.tax, "tax on {invoice}, {ctx}");

        self.settlements.clear();
        self.last_bill = Some(at);
        // Included credit is minted as the invoice finalizes, after its own draw, effective from
        // the start of the period it is included for.
        for (component, amount, denomination, expiry, rollover) in included {
            let expires = match expiry {
                CreditExpiry::BillingPeriod => Some(period.1),
                other => other.resolve(period.0.and_utc()).map(|t| t.naive_utc()),
            };
            self.grant(
                GrantRef::PlanCredit { invoice, component },
                denomination,
                amount,
                DEFAULT_PRIORITY,
                period.0,
                expires,
                true,
                rollover,
            );
        }
        self.invoices.push((
            invoice,
            Invoice {
                issued_at: at,
                period,
                due,
                paid: false,
                packs: paid_packs,
            },
        ));
        if due.is_zero() {
            self.settle(invoice, at);
        }
    }

    /// Checks the balance at the moments the history names, as they come.
    fn probe(&mut self, probes: &mut Vec<(NaiveDateTime, Probe)>, until: NaiveDateTime) {
        while let Some((at, probe)) = probes.first().copied() {
            if at > until {
                return;
            }
            probes.remove(0);
            self.sweep(at);
            let ctx = self.context(at);
            match probe {
                Probe::LowBalance => {
                    let threshold = *self.low_balance.first().expect("a low-balance threshold");
                    assert_eq!(self.balance(at), threshold, "low balance, {ctx}");
                }
                Probe::Paused => {
                    assert_eq!(self.balance(at), Decimal::ZERO, "paused, {ctx}");
                    let after = self.usage_between(at, self.clock.as_of);
                    assert_eq!(after, Decimal::ZERO, "usage after the pause, {ctx}");
                }
            }
        }
    }

    fn run(mut self) {
        let events = resolve_events(self.history, &self.clock);
        let expected = &self.history.expected;
        let mut probes: Vec<(NaiveDateTime, Probe)> = expected
            .low_balance_at
            .map(|at| (self.clock.resolve(at), Probe::LowBalance))
            .into_iter()
            .chain(
                expected
                    .usage_paused_at
                    .map(|at| (self.clock.resolve(at), Probe::Paused)),
            )
            .collect();
        probes.sort_by_key(|p| p.0);
        for (at, action) in events {
            self.probe(&mut probes, at);
            self.sweep(at);
            let ctx = self.context(at);
            match action {
                Action::Subscribe {
                    plan,
                    billing_day_anchor,
                    ..
                } => {
                    assert_eq!(plan, self.plan.name);
                    assert_eq!(billing_day_anchor, Some(self.clock.billing_day as u16));
                }
                Action::AddCard {
                    card,
                    spec,
                    make_default,
                } => {
                    self.cards.insert(card, spec);
                    if make_default {
                        self.default_card = Some(card);
                    }
                }
                Action::RemoveCard { card } => {
                    self.cards.remove(card);
                    assert_ne!(self.default_card, Some(card), "{ctx}");
                }
                Action::SetDefaultCard { card } => {
                    assert!(self.cards.contains_key(card), "{ctx}");
                    self.default_card = Some(card);
                }
                Action::BillCycle { invoice, expected } => self.bill(invoice, at, &expected),
                Action::Purchase {
                    purchase,
                    denomination,
                    amount,
                    expires_in_months,
                } => {
                    self.purchases
                        .insert(purchase, (denomination, amount, expires_in_months));
                }
                Action::Charge { payable, outcome } => {
                    assert_eq!(outcome, self.charge_outcome(), "charge of {payable}, {ctx}");
                    if outcome == ChargeOutcome::Paid {
                        self.settle(payable, at);
                    }
                }
                Action::AutoTopUp {
                    purchase,
                    denomination,
                    amount,
                    expires_in_months,
                    outcome,
                } => {
                    let rule = self.auto_top_up.clone().expect("an automatic top-up rule");
                    assert!(!self.suspended, "{ctx}");
                    let balance = self.balance(at);
                    assert_eq!(balance, rule.threshold, "top-up {purchase} fires at, {ctx}");
                    let expected_amount = match rule.amount {
                        AutoTopUpAmount::Fixed(fixed) => fixed,
                        AutoTopUpAmount::ToTarget(target) => target - balance,
                    };
                    assert_eq!(amount, expected_amount, "top-up {purchase}, {ctx}");
                    assert_eq!(outcome, self.charge_outcome(), "top-up {purchase}, {ctx}");
                    match outcome {
                        ChargeOutcome::Paid => {
                            self.purchases
                                .insert(purchase, (denomination, amount, expires_in_months));
                            self.settle(purchase, at);
                        }
                        ChargeOutcome::Declined(_) => self.suspended = true,
                    }
                }
                Action::BankTransfer { payable } => self.settle(payable, at),
                Action::RedeemPromoCode { code } => {
                    let promo = self
                        .scenario
                        .promo_codes
                        .iter()
                        .find(|p| p.code == code)
                        .expect("a scenario promotion code");
                    let expires = CreditExpiry::Day(promo.expires_after_days)
                        .resolve(at.and_utc())
                        .map(|t| t.naive_utc());
                    self.grant(
                        GrantRef::PromoCode(code),
                        promo.currency,
                        promo.amount,
                        DEFAULT_PRIORITY,
                        at,
                        expires,
                        true,
                        None,
                    );
                }
                Action::AddCredit {
                    denomination,
                    amount,
                    reason,
                    promotional,
                    expires,
                } => {
                    let expires = expires.map(|w| self.clock.resolve(w));
                    self.grant(
                        GrantRef::Added(reason),
                        denomination,
                        amount,
                        DEFAULT_PRIORITY,
                        at,
                        expires,
                        promotional,
                        None,
                    );
                }
                Action::ConfigureWallet {
                    low_balance_thresholds,
                    auto_top_up,
                    ..
                } => {
                    self.low_balance = low_balance_thresholds;
                    self.auto_top_up = auto_top_up;
                }
                Action::SetEntitlementOverride(_) | Action::SetBudget(_) => {}
            }
        }
        self.probe(&mut probes, self.clock.as_of);
        assert!(probes.is_empty());
        self.check_books();
    }

    fn check_books(mut self) {
        let as_of = self.clock.as_of;
        self.sweep(as_of);
        let ctx = self.context(as_of);
        let expected = &self.history.expected;

        let mut denominations: Vec<&str> = self
            .grants
            .iter()
            .filter(|g| !g.remaining.is_zero())
            .map(|g| g.denomination)
            .collect();
        denominations.sort();
        denominations.dedup();
        let mut wanted: Vec<&str> = expected.wallets.iter().map(|w| w.denomination).collect();
        wanted.sort();
        assert_eq!(denominations, wanted, "wallets, {ctx}");

        for wallet in &expected.wallets {
            let on_account = self.on_account(wallet.denomination);
            let unbilled = if wallet.denomination == self.pricing {
                self.unbilled(as_of)
            } else {
                Decimal::ZERO
            };
            assert_eq!(on_account, wallet.on_account, "on account, {ctx}");
            assert_eq!(unbilled, wallet.unbilled, "unbilled, {ctx}");
            assert_eq!(on_account - unbilled, wallet.balance, "balance, {ctx}");
            let mut live: Vec<(GrantRef, Decimal, Option<NaiveDateTime>)> = self
                .grants
                .iter()
                .filter(|g| g.denomination == wallet.denomination && !g.remaining.is_zero())
                .map(|g| (g.source, g.remaining, g.expires_at))
                .collect();
            let mut want: Vec<(GrantRef, Decimal, Option<NaiveDateTime>)> = wallet
                .grants
                .iter()
                .map(|g| {
                    (
                        g.source,
                        g.remaining,
                        g.expires.and_then(|e| e.resolve(&self.clock)),
                    )
                })
                .collect();
            let key = |g: &(GrantRef, Decimal, Option<NaiveDateTime>)| format!("{:?}", g.0);
            live.sort_by_key(key);
            want.sort_by_key(key);
            assert_eq!(live, want, "grants, {ctx}");
        }

        assert_eq!(self.suspended, expected.auto_top_up_suspended, "{ctx}");
        if let Some(at) = expected.limit_reached_at {
            let at = self.clock.resolve(at);
            let limit = self
                .plan
                .entitlements
                .iter()
                .find_map(|e| e.limit())
                .expect("a token limit");
            let start = self.clock.cycle_start(0).and_time(NaiveTime::MIN);
            let tokens: u64 = self
                .windows
                .iter()
                .filter(|w| w.from >= start && w.to <= at)
                .map(|w| w.tokens)
                .sum();
            assert_eq!(Decimal::from(tokens), limit, "limit, {ctx}");
            assert!(
                self.windows.iter().all(|w| w.to <= at),
                "usage after the limit, {ctx}"
            );
        }
        assert!(
            self.invoices
                .iter()
                .all(|(_, i)| i.paid || !i.due.is_zero()),
            "an invoice with nothing due is settled, {ctx}"
        );
        let open: Vec<(&str, NaiveDateTime)> = self
            .invoices
            .iter()
            .filter(|(_, i)| !i.paid)
            .map(|(h, i)| {
                (
                    *h,
                    i.issued_at + Duration::days(i64::from(self.plan.payment.net_terms)),
                )
            })
            .collect();
        let want: Vec<(&str, NaiveDateTime)> = expected
            .open_invoices
            .iter()
            .map(|(h, due)| (*h, self.clock.resolve(*due)))
            .collect();
        assert_eq!(open, want, "open invoices, {ctx}");
        assert!(self.purchases.is_empty(), "unpaid purchases, {ctx}");
    }
}

#[test]
fn the_simulation_catches_a_wrong_balance() {
    let mut scenario = ai_credits_scenario();
    let tessera = scenario
        .customers
        .iter_mut()
        .find(|c| c.name == "Tessera Health")
        .unwrap();
    tessera.history.as_mut().unwrap().expected.wallets[0].on_account += dec!(1);
    let (customer, history) = histories(&scenario)
        .into_iter()
        .find(|(c, _)| c.name == "Tessera Health")
        .unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Simulation::new(&scenario, customer, history, reference_as_of()).run()
    }));
    assert!(result.is_err());
}

#[test]
fn reference_dates_match_the_story() {
    let scenario = ai_credits_scenario();
    let date = |name: &str, when: When| {
        let history = scenario
            .customers
            .iter()
            .find(|c| c.name == name)
            .unwrap()
            .history
            .as_ref()
            .unwrap();
        Clock::new(reference_as_of(), history.cycle_age_days)
            .resolve(when)
            .date()
    };
    let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
    assert_eq!(date("Tessera Health", cycle(3)), d(6, 29));
    assert_eq!(date("Brightloom Labs", cycle(4)), d(5, 11));
    assert_eq!(date("Brightloom Labs", cycle_at(3, -3, 10, 0)), d(6, 8));
    assert_eq!(date("Brightloom Labs", cycle_at(2, 6, 10, 0)), d(7, 17));
    assert_eq!(date("Quillbase", cycle_at(2, 21, 0, 0)), d(8, 24));
    assert_eq!(date("Nightjar Robotics", cycle(1)), d(8, 17));
    assert_eq!(date("Ferrous Logistics", cycle_at(1, 17, 11, 0)), d(8, 31));
    assert_eq!(date("Ferrous Logistics", cycle_at(0, 2, 10, 0)), d(9, 16));
    assert_eq!(date("Lumen Notes", cycle_at(0, 7, 11, 0)), d(9, 22));
    assert_eq!(date("Lumen Notes", cycle_at(-4, -15, 0, 0)), d(12, 31));
    assert_eq!(date("Wren Labs", cycle_at(0, 5, 16, 40)), d(10, 6));
    assert_eq!(date("Meridian Freight", cycle(6)), d(4, 1));
    assert_eq!(
        date("Meridian Freight", cycle_at(-6, -1, 0, 0)),
        NaiveDate::from_ymd_opt(2027, 3, 31).unwrap()
    );
    assert_eq!(date("Meridian Freight", cycle_at(0, 30, 0, 0)), d(10, 31));
}

#[test]
fn a_rolled_grant_draws_before_credit_expiring_sooner() {
    let scenario = ai_credits_scenario();
    let (customer, history) = histories(&scenario)
        .into_iter()
        .find(|(c, _)| c.name == "Lumen Notes")
        .unwrap();
    let mut sim = Simulation::new(&scenario, customer, history, reference_as_of());
    let now = reference_as_of();
    let rolled = GrantRef::Rollover {
        invoice: "inv-2",
        component: INCLUDED_CREDITS,
    };
    sim.grant(
        GrantRef::Added("Goodwill"),
        CREDITS,
        dec!(300),
        DEFAULT_PRIORITY,
        now - Duration::days(1),
        Some(now + Duration::days(2)),
        true,
        None,
    );
    sim.grant(
        rolled,
        CREDITS,
        dec!(620),
        DEFAULT_PRIORITY - 1,
        now - Duration::days(1),
        Some(now + Duration::days(7)),
        true,
        None,
    );
    // Not spendable at the instant it expires.
    sim.grant(
        GrantRef::Added("Lapsing"),
        CREDITS,
        dec!(50),
        DEFAULT_PRIORITY - 1,
        now - Duration::days(10),
        Some(now),
        true,
        None,
    );
    let applied = sim.apply(CREDITS, dec!(700), now);
    assert_eq!(
        applied,
        vec![(rolled, dec!(620)), (GrantRef::Added("Goodwill"), dec!(80))]
    );
}

#[test]
fn expiring_credit_pays_the_usage_it_covered_before_lapsing() {
    let scenario = ai_credits_scenario();
    let expiry = cycle_at(1, 10, 0, 0);
    let paid = cycle_at(1, 0, 10, 1);
    let history = History {
        cycle_age_days: 5,
        sales_tax: None,
        events: vec![
            ev(
                cycle(1).plus_minutes(-10),
                add_card("visa", card("visa", "4242", CardBehavior::Approved)),
            ),
            ev(cycle(1), subscribe(PAY_AS_YOU_GO)),
            ev(cycle(1), bill("inv-1", InvoiceExpectation::empty())),
            ev(
                cycle_at(1, 0, 9, 0),
                Action::AddCredit {
                    denomination: USD,
                    amount: dec!(50),
                    reason: "Goodwill",
                    promotional: true,
                    expires: Some(expiry),
                },
            ),
            ev(cycle_at(1, 0, 10, 0), purchase("P1", dec!(500))),
            ev(paid, charge("P1")),
            ev(
                cycle(0),
                usage_bill(
                    "inv-2",
                    dec!(120),
                    vec![
                        (GrantRef::Added("Goodwill"), dec!(30)),
                        (GrantRef::Purchase("P1"), dec!(90)),
                    ],
                ),
            ),
        ],
        usage: usage_plan(
            "brightloom-labs",
            1.0,
            vec![
                UsageTarget::new(cycle(1), cycle(0), UsageAmount::Priced(dec!(120)))
                    .with_checkpoint(expiry, dec!(30)),
            ],
        ),
        expected: Expected {
            wallets: vec![usd_wallet(
                dec!(410),
                Decimal::ZERO,
                vec![grant(
                    GrantRef::Purchase("P1"),
                    dec!(410),
                    months_after(paid, PURCHASE_TERM_MONTHS),
                )],
            )],
            ..Expected::default()
        },
    };
    let customer = account("Settled Labs", "Austin", "TX", history);
    let history = customer.history.as_ref().unwrap();
    Simulation::new(&scenario, &customer, history, reference_as_of()).run();
}

#[test]
fn every_component_maps_to_an_engine_valid_price() {
    use common_domain::ids::{BaseId, BillableMetricId, CreditUnitId, ProductFamilyId, TenantId};
    use meteroid_store::domain::price_components::PriceEntry;
    use meteroid_store::domain::prices::{Pricing, validate_price_credit_unit, validate_pricing};

    let scenario = ai_credits_scenario();
    let spec = &scenario.metrics[0];
    let metric = meteroid_store::domain::BillableMetric {
        id: BillableMetricId::new(),
        name: spec.name.clone(),
        description: None,
        code: spec.code.clone(),
        aggregation_type: spec.aggregation_type,
        aggregation_key: spec.aggregation_key.clone(),
        unit_conversion_factor: spec.unit_conversion_factor,
        unit_conversion_rounding: spec.unit_conversion_rounding.clone(),
        segmentation_matrix: spec.segmentation_matrix.clone(),
        filters: None,
        usage_group_key: None,
        unit_singular: spec
            .unit_label
            .as_ref()
            .map(|label| label.singular.to_owned()),
        unit_plural: spec
            .unit_label
            .as_ref()
            .map(|label| label.plural.to_owned()),
        created_at: NaiveDateTime::default(),
        updated_at: None,
        archived_at: None,
        tenant_id: TenantId::new(),
        product_family_id: ProductFamilyId::new(),
        product_id: None,
        usage_scope: Default::default(),
    };
    let units = HashMap::from([(CREDITS.to_string(), CreditUnitId::new())]);
    let mut once = vec![];
    for plan in &scenario.plans {
        for component in &plan.components {
            let internal = component
                .to_domain(std::slice::from_ref(&metric), &plan.currency, &units)
                .unwrap_or_else(|e| panic!("{} of {}: {e:?}", component.name, plan.name));
            for price in &internal.prices {
                let PriceEntry::New(input) = price else {
                    panic!("a new price for {}", component.name);
                };
                validate_pricing(&input.pricing, &plan.currency).unwrap();
                validate_price_credit_unit(&input.pricing, input.credit_unit.as_ref()).unwrap();
                if let Pricing::Credit { once: true, .. } = &input.pricing {
                    once.push((plan.name.as_str(), component.name.as_str()));
                }
                if plan.name == STUDIO {
                    match &input.pricing {
                        Pricing::Credit {
                            credit_unit_id,
                            rollover,
                            unit_price,
                            ..
                        } => assert!(
                            credit_unit_id.is_some() && rollover.is_some() && unit_price.is_zero()
                        ),
                        Pricing::Usage(_) => assert!(input.credit_unit.is_some()),
                        Pricing::Rate { rate } => assert_eq!(*rate, dec!(49)),
                        other => panic!("unexpected Studio price {other:?}"),
                    }
                }
            }
        }
    }
    assert_eq!(once, vec![(SCALE, WELCOME_CREDIT)]);
}
