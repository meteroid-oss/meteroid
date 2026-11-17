//! Generated stories run as the controller runs them, each pass issuing what the open documents
//! owe beyond the lines issued so far, checked against `reference` and conservation rules.
//! `BILLING_STORIES` sets the case count, `BILLING_STORIES_SEED` the seed.

mod events;
mod reference;
mod story;

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use self::events::Events;
use self::reference::Reference;
use self::story::{At, Built, Kind, Price, Story, build, metric, midnight, story};
use crate::charge::{ChargeKey, ChargeKind, Denomination, Fee};
use crate::evaluate::{Charge, DocumentSlot, Evaluation, evaluate};
use crate::facts::Facts;
use crate::issued::{IssuedKind, IssuedLine};
use crate::pricer::{CreditGrant, Pricer, Rate, Segment, Shown, UsageReads, UsageWindow};
use crate::pricing::PricingError;
use crate::reconcile::{Adjustment, Expected, TakeBack, reconcile};

/// Prices a story's fees: a seat fee per seat, a pack's grant its fee, usage per unit.
struct Book;

impl Pricer<Price> for Book {
    fn metrics(&self, fee: &Price) -> Vec<common_domain::ids::BillableMetricId> {
        (fee.kind == Kind::Usage).then(metric).into_iter().collect()
    }

    fn rates(&self, segment: &Segment<'_, Price>) -> Result<Vec<Rate>, PricingError> {
        Ok(segment
            .fees
            .iter()
            .filter(|fee| fee.fee.kind != Kind::Usage)
            .map(|fee| {
                let seats = if fee.fee.kind == Kind::Seats {
                    fee.seats.unwrap_or(1)
                } else {
                    1
                };
                let amount = fee.fee.amount * i64::try_from(seats).expect("seats");
                let pack = matches!(fee.fee.kind, Kind::Pack | Kind::BoughtPack);
                let prorated = fee.fee.kind.prorated();
                Rate {
                    fee: fee.lineage.clone(),
                    kind: if pack {
                        ChargeKind::Credit
                    } else {
                        ChargeKind::Fee
                    },
                    quantity: Some(Decimal::from(seats)),
                    amount,
                    denomination: Denomination::Money,
                    prorated,
                    credit_grant: pack.then_some(CreditGrant {
                        amount,
                        denomination: Denomination::Money,
                        prorated,
                    }),
                    once_per_product: None,
                    shown: Shown::default(),
                }
            })
            .collect())
    }

    fn usage(
        &self,
        segment: &Segment<'_, Price>,
        window: &UsageWindow,
        reads: &UsageReads,
    ) -> Result<Vec<Rate>, PricingError> {
        Ok(segment
            .fees
            .iter()
            .filter(|fee| fee.fee.kind == Kind::Usage)
            .filter_map(|fee| {
                let usage = reads.get(&crate::pricer::UsageRead {
                    metric: metric(),
                    window: *window,
                })?;
                let units: Decimal = usage.iter().map(|(_, quantity)| *quantity).sum();
                Some(Rate {
                    fee: fee.lineage.clone(),
                    kind: ChargeKind::Usage {
                        metric: metric(),
                        dimensions: BTreeMap::new(),
                    },
                    quantity: Some(units),
                    amount: (units * Decimal::from(fee.fee.amount))
                        .to_i64()
                        .expect("an amount"),
                    denomination: Denomination::Money,
                    prorated: false,
                    credit_grant: None,
                    once_per_product: None,
                    shown: Shown::default(),
                })
            })
            .collect())
    }
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| {
            v.strip_prefix("0x")
                .map(|hex| u64::from_str_radix(hex, 16).ok())
                .unwrap_or_else(|| v.parse().ok())
        })
        .unwrap_or(default)
}

fn seed() -> u64 {
    env("BILLING_STORIES_SEED", 0x5eed_b111)
}

/// A fixed handful of cases on every run, from a fixed seed; more behind `BILLING_STORIES`.
fn config() -> Config {
    Config {
        cases: u32::try_from(env("BILLING_STORIES", 48)).unwrap_or(48),
        rng_seed: RngSeed::Fixed(seed()),
        failure_persistence: None,
        max_shrink_iters: 4_096,
        ..Config::default()
    }
}

/// Whether a document of `slot` is issued by `now`.
fn issued_by(slot: DocumentSlot, now: At, facts: &Facts<Price>) -> bool {
    match slot {
        DocumentSlot::Recurring { period_start } => midnight(period_start) <= now,
        DocumentSlot::Adjustment { on } => midnight(on) <= now,
        DocumentSlot::Final => facts
            .effective_ending()
            .is_some_and(|ending| ending.at <= now),
    }
}

/// The day a document of `slot` is issued.
fn issued_on(slot: DocumentSlot, facts: &Facts<Price>) -> Option<NaiveDate> {
    match slot {
        DocumentSlot::Recurring { period_start } => Some(period_start),
        DocumentSlot::Adjustment { on } => Some(on),
        DocumentSlot::Final => facts.effective_ending().map(|ending| ending.on()),
    }
}

/// What a line still bills, notes taken off.
fn net(line: &IssuedLine) -> i64 {
    line.subtotal - line.corrected
}

struct Step {
    at: At,
    adjustments: Vec<Adjustment>,
}

struct Sim<'a> {
    built: &'a Built,
    events: Events,
    lines: Vec<IssuedLine>,
    /// The day each line is dated: the day it was billed, within its charge's period.
    dated: Vec<NaiveDate>,
    /// The lines of each document, in issue order.
    documents: Vec<Vec<usize>>,
    voided: bool,
    steps: Vec<Step>,
    /// The lines the last pass found settled by the watermark, owned by no charge.
    settled: Vec<String>,
    /// Each take-back: the line it credits, the day and the amount.
    notes: Vec<(String, NaiveDate, i64)>,
    names: &'a BTreeMap<Fee, String>,
}

/// The charge each line counts for: the narrowest it bills, as a change reads it, so a month's
/// line under a year's charge counts once.
fn owners(lines: &[IssuedLine], expected: &[Expected], charges: &[Charge]) -> Vec<Option<usize>> {
    lines
        .iter()
        .map(|line| {
            let owner = crate::evaluate::owner(line, expected.iter().map(|e| &e.key), charges)?;
            expected.iter().position(|e| e.key == *owner)
        })
        .collect()
}

impl<'a> Sim<'a> {
    fn new(built: &'a Built) -> Self {
        let start = midnight(built.snapshots[0].1.start.date);
        Sim {
            built,
            events: Events::new(start, built.until + Duration::days(2)),
            lines: Vec::new(),
            dated: Vec::new(),
            documents: Vec::new(),
            voided: false,
            steps: Vec::new(),
            settled: Vec::new(),
            notes: Vec::new(),
            names: &built.names,
        }
    }

    fn facts(&self, now: At) -> Facts<Price> {
        let mut facts = self.built.facts_at(now).clone();
        if let Some((day, set)) = self.built.watermark
            && set <= now
        {
            facts.reconcile_from = Some(day);
        }
        facts
    }

    fn evaluate(&self, facts: &Facts<Price>, now: At) -> Result<Evaluation, String> {
        let first = evaluate(facts, now, None, &Book).map_err(|e| format!("{e:?}"))?;
        let reads = self.events.reads(&first.usage_needed, now);
        evaluate(facts, now, Some(&reads), &Book).map_err(|e| format!("{e:?}"))
    }

    fn void(&mut self, pick: u8) {
        let candidates: Vec<usize> = (0..self.documents.len())
            .filter(|&d| {
                self.documents[d]
                    .iter()
                    .any(|&l| !self.lines[l].voided && self.lines[l].subtotal > 0)
            })
            .collect();
        if candidates.is_empty() {
            return;
        }
        let document = candidates[usize::from(pick) % candidates.len()];
        for &l in &self.documents[document] {
            self.lines[l].voided = true;
        }
        self.voided = true;
    }

    /// One controller pass at `now`, not applied yet: the open documents' charges, and the zero a
    /// fixed line is owed once its charge left the facts (as a change reconciles it), against what
    /// was issued.
    #[allow(clippy::type_complexity)]
    fn plan(
        &self,
        now: At,
    ) -> Result<(Evaluation, Vec<Expected>, Vec<Adjustment>, Vec<String>), String> {
        let facts = self.facts(now);
        let evaluation = self.evaluate(&facts, now)?;
        let all = evaluation.expected();
        let mut expected: Vec<Expected> = Vec::new();
        for charge in &evaluation.charges {
            if charge.is_commitment() || !issued_by(charge.document, now, &facts) {
                continue;
            }
            match expected.iter_mut().find(|e| e.key == charge.key) {
                Some(e) => e.amount += charge.amount,
                None => expected.push(Expected {
                    key: charge.key.clone(),
                    amount: charge.amount,
                }),
            }
        }
        let watermark = facts.reconcile_from;
        for (line, dated) in self.lines.iter().zip(&self.dated) {
            let gone = !line.voided
                && line.key.kind == ChargeKind::Fee
                // R-094: a line dated before the watermark settled what was due then.
                && watermark.is_none_or(|w| !self.behind(&line.key, w) && *dated >= w)
                && !all.iter().any(|e| line.bills(&e.key))
                && !expected.iter().any(|e| e.key == line.key);
            if gone {
                expected.push(Expected {
                    key: line.key.clone(),
                    amount: 0,
                });
            }
        }
        let take_back = if self.built.unused_first {
            TakeBack::Unused(now.date_naive())
        } else {
            TakeBack::LatestFirst
        };
        // R-094: a line of a period the watermark settled is no other charge's.
        let settled: Vec<String> = self
            .lines
            .iter()
            .filter(|line| evaluation.settles(line))
            .map(|line| line.id.clone())
            .collect();
        let owners = self.owners(&expected, &settled, &evaluation.charges);
        let mut adjustments: Vec<Adjustment> = Vec::new();
        for (i, charge) in expected.iter().enumerate() {
            // R-103: a pack is never given back by reconcile; its credit's settlement does that.
            let pack = charge.key.kind == ChargeKind::Credit;
            let owned = self.owned(i, &owners);
            let mut made = reconcile(std::slice::from_ref(charge), &owned, take_back);
            // R-094: what the lines dated from the watermark bill caps a bill, never a take-back.
            if let Some(w) = watermark.filter(|w| self.legacy(&owned, Some(*w))) {
                let since: i64 = evaluation
                    .charges
                    .iter()
                    .filter(|c| {
                        c.key == charge.key
                            && !c.is_commitment()
                            && issued_by(c.document, now, &facts)
                            && c.due_at >= midnight(w)
                    })
                    .map(|c| c.amount)
                    .sum();
                // What the lines dated from the watermark bill, less the notes from it on
                // earlier lines.
                let since_billed: i64 = owned
                    .iter()
                    .map(|line| match self.dated_of(line) {
                        Some(d) if d >= w => line.subtotal - line.corrected,
                        _ => -self.noted_from(line, w),
                    })
                    .sum();
                let cap = since - since_billed;
                for a in &mut made {
                    if let Adjustment::Bill { amount, .. } = a {
                        *amount = (*amount).min(cap);
                    }
                }
                made.retain(|a| !matches!(a, Adjustment::Bill { amount, .. } if *amount <= 0));
            }
            adjustments.extend(
                made.into_iter()
                    .filter(|a| !pack || matches!(a, Adjustment::Bill { .. })),
            );
        }
        Ok((evaluation, expected, adjustments, settled))
    }

    /// What notes dated from `w` took back from `line`.
    fn noted_from(&self, line: &IssuedLine, w: NaiveDate) -> i64 {
        self.notes
            .iter()
            .filter(|(id, day, _)| *id == line.id && *day >= w)
            .map(|(_, _, amount)| amount)
            .sum()
    }

    fn dated_of(&self, line: &IssuedLine) -> Option<NaiveDate> {
        let i = self.lines.iter().position(|l| l.id == line.id)?;
        self.dated.get(i).copied()
    }

    /// Whether one of a charge's `owned` lines is dated before the watermark `w`: it settled what
    /// the charge owed before `w`, so only what falls due from `w` is billed, against the lines
    /// and notes dated from it (R-094).
    fn legacy(&self, owned: &[IssuedLine], w: Option<NaiveDate>) -> bool {
        w.is_some_and(|w| {
            owned
                .iter()
                .any(|line| self.dated_of(line).is_some_and(|d| d < w))
        })
    }

    /// Each line's charge, none for the `settled` ones.
    fn owners(
        &self,
        expected: &[Expected],
        settled: &[String],
        charges: &[Charge],
    ) -> Vec<Option<usize>> {
        owners(&self.lines, expected, charges)
            .into_iter()
            .zip(&self.lines)
            .map(|(owner, line)| owner.filter(|_| !settled.contains(&line.id)))
            .collect()
    }

    /// The lines whose narrowest charge is the `i`-th.
    fn owned(&self, i: usize, owners: &[Option<usize>]) -> Vec<IssuedLine> {
        self.lines
            .iter()
            .zip(owners)
            .filter(|(_, owner)| **owner == Some(i))
            .map(|(line, _)| line.clone())
            .collect()
    }

    fn step(&mut self, now: At) -> Result<(Evaluation, Vec<Expected>), String> {
        let (evaluation, expected, adjustments, settled) = self.plan(now)?;
        self.settled = settled;
        let facts = self.facts(now);
        self.apply(&adjustments, now.date_naive(), &evaluation.charges, &facts);
        self.steps.push(Step {
            at: now,
            adjustments,
        });
        Ok((evaluation, expected))
    }

    fn apply(
        &mut self,
        adjustments: &[Adjustment],
        today: NaiveDate,
        charges: &[Charge],
        facts: &Facts<Price>,
    ) {
        let mut document: Vec<usize> = Vec::new();
        for adjustment in adjustments {
            match adjustment {
                Adjustment::Bill { key, amount } => {
                    // Dated as the store renders it: over the windows of the key's charges the
                    // documents issued today carry, whenever they fell due.
                    let due: Vec<(NaiveDate, NaiveDate)> = charges
                        .iter()
                        .filter(|c| c.key == *key && issued_on(c.document, facts) == Some(today))
                        .map(|c| c.window)
                        .collect();
                    let from = due.iter().map(|w| w.0).min().unwrap_or(key.from);
                    let until = due.iter().map(|w| w.1).max().unwrap_or(key.until);
                    document.push(self.lines.len());
                    self.dated.push(today.clamp(key.from, key.until));
                    self.lines.push(IssuedLine {
                        id: format!("l{}", self.lines.len()),
                        key: key.clone(),
                        kind: IssuedKind::Fee {
                            price_component: None,
                            add_on: false,
                        },
                        lineage: None,
                        quantity: None,
                        unit_priced: false,
                        from,
                        until,
                        subtotal: *amount,
                        quoted: None,
                        already_credited: 0,
                        corrected: 0,
                        settled: None,
                        voided: false,
                    });
                }
                Adjustment::Credit { line, amount } => {
                    if let Some(found) = self.lines.iter_mut().find(|l| l.id == *line) {
                        found.already_credited += amount;
                        found.corrected += amount;
                        self.notes.push((line.clone(), today, *amount));
                    }
                }
                Adjustment::Short { .. } => {}
            }
        }
        if !document.is_empty() {
            self.documents.push(document);
        }
    }

    /// Whether the charge `key` is behind the watermark `w` (R-094): its period ends by it. A
    /// charge of a day, keyed [day, day), is due that day; a usage key is a part of its period,
    /// behind on the watermark when it ends one.
    fn behind(&self, key: &ChargeKey, w: NaiveDate) -> bool {
        let usage = matches!(key.kind, ChargeKind::Usage { .. });
        key.until < w
            || (key.until == w
                && key.from < w
                && (!usage || self.boundary_of(&key.fee, key.from, w)))
    }

    /// Whether `day` is a boundary of the grid the row of `lineage` in force on `on` renews on:
    /// its cadence, from the plan window in force when its run started, the rows before it of
    /// its lineage billing alike and meeting it (R-084).
    fn boundary_of(&self, lineage: &Fee, on: NaiveDate, day: NaiveDate) -> bool {
        let facts = self.built.last();
        facts
            .fee_on(lineage, on)
            .and_then(|row| {
                let mut start = row.from;
                while let Some(before) = facts.fees.iter().find(|other| {
                    other.lineage == *lineage
                        && other.to == Some(start)
                        && other.from < start
                        && other.billing == row.billing
                        && other.cadence == row.cadence
                }) {
                    start = before.from;
                }
                let window = facts.plan_on(start)?;
                story::grid_for(facts, window.cadence_origin, row.cadence)
            })
            .is_some_and(|grid| grid.is_boundary(day))
    }

    fn describe(&self, adjustment: &Adjustment) -> String {
        match adjustment {
            Adjustment::Bill { key, amount } => format!(
                "bill {} {:?} {}..{} {amount}",
                self.name(&key.fee),
                key.kind,
                key.from,
                key.until
            ),
            Adjustment::Credit { line, amount } => {
                let key = self.lines.iter().find(|l| l.id == *line).map(|l| &l.key);
                format!(
                    "credit {line} ({}) {amount}",
                    key.map(|key| format!("{} {}..{}", self.name(&key.fee), key.from, key.until))
                        .unwrap_or_default()
                )
            }
            Adjustment::Short { key, amount } => format!(
                "short {} {}..{} {amount}",
                self.name(&key.fee),
                key.from,
                key.until
            ),
        }
    }

    fn name(&self, fee: &Fee) -> String {
        self.names
            .get(fee)
            .cloned()
            .unwrap_or_else(|| format!("{fee:?}"))
    }

    fn report(&self, failure: &str) -> String {
        let mut out = vec![
            failure.to_owned(),
            format!(
                "replay with BILLING_STORIES_SEED={:#x} (the minimal story is printed below)",
                seed()
            ),
            "story:".to_owned(),
        ];
        out.extend(self.built.narrative.iter().map(|line| format!("  {line}")));
        out.push("documents:".to_owned());
        for step in self.steps.iter().filter(|s| !s.adjustments.is_empty()) {
            let what: Vec<String> = step.adjustments.iter().map(|a| self.describe(a)).collect();
            out.push(format!(
                "  {}: {}",
                step.at.format("%Y-%m-%d %H:%M"),
                what.join("; ")
            ));
        }
        out.join("\n")
    }

    /// Every instant to run: each one a pass names as its next, each decision, void and the
    /// watermark, up to the story's last instant.
    fn run(&mut self) -> Result<(Evaluation, Vec<Expected>), String> {
        let built = self.built;
        let mut marks: Vec<At> = built
            .snapshots
            .iter()
            .map(|(at, _)| *at)
            .chain(built.voids.iter().map(|(at, _)| *at))
            .chain(
                built
                    .watermark
                    .iter()
                    .flat_map(|(day, set)| [midnight(*day), *set]),
            )
            .chain(std::iter::once(built.until))
            .collect();
        marks.sort();
        marks.dedup();
        let mut voids = built.voids.clone();
        voids.sort_by_key(|(at, _)| *at);
        let mut voids = voids.into_iter().peekable();
        let mut now = marks[0];
        for _ in 0..5_000 {
            while let Some((_, pick)) = voids.next_if(|(at, _)| *at <= now) {
                self.void(pick);
            }
            let (evaluation, expected) = self.step(now)?;
            self.check_step(now, &expected, &evaluation.charges)?;
            if now >= built.until {
                return Ok((evaluation, expected));
            }
            let next_mark = marks.iter().copied().find(|at| *at > now);
            now = evaluation
                .next_wake
                .into_iter()
                .chain(next_mark)
                .filter(|at| *at > now)
                .min()
                .unwrap_or(built.until)
                .min(built.until);
        }
        Err("the controller never reached the story's end".to_owned())
    }

    /// After a pass: a second pass changes nothing; each charge's live lines bill what it owes
    /// beyond what a void forgave, at most what it owes, never below nothing, a pack at least that
    /// (R-080, R-096, R-103);
    /// no line is credited beyond what it billed; nothing behind the watermark moves (R-094).
    fn check_step(&self, now: At, expected: &[Expected], charges: &[Charge]) -> Result<(), String> {
        let shorts: Vec<&ChargeKey> = self
            .steps
            .last()
            .map(|step| {
                step.adjustments
                    .iter()
                    .filter_map(|a| match a {
                        Adjustment::Short { key, .. } => Some(key),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let owners = self.owners(expected, &self.settled, charges);
        for (i, charge) in expected.iter().enumerate() {
            if shorts.contains(&&charge.key) {
                continue;
            }
            let owned = self.owned(i, &owners);
            let live: i64 = owned.iter().filter(|l| !l.voided).map(net).sum();
            let voided: i64 = owned.iter().filter(|l| l.voided).map(net).sum();
            let owed = (charge.amount - voided).max(0);
            let pack = charge.key.kind == ChargeKind::Credit;
            // A take-back reads voided lines as standing and forgives their share (R-096): live
            // lines keep what the charge owes less what of the voided lines it still holds.
            let forgiven = voided > 0 && (owed..=charge.amount.max(0)).contains(&live);
            let settled = live < owed && self.legacy(&owned, self.facts(now).reconcile_from);
            if !(live == owed || forgiven || settled || (pack && live > owed)) {
                return Err(format!(
                    "R-080/R-096 at {now}: {} {:?} {}..{} owes {} with {voided} voided, its live lines bill {live}",
                    self.name(&charge.key.fee),
                    charge.key.kind,
                    charge.key.from,
                    charge.key.until,
                    charge.amount
                ));
            }
        }
        if let Some(line) = self.lines.iter().find(|l| l.already_credited > l.subtotal) {
            return Err(format!(
                "R-080 at {now}: line {} billed {} and was credited {}",
                line.id, line.subtotal, line.already_credited
            ));
        }
        let last = self
            .steps
            .last()
            .map(|s| s.adjustments.clone())
            .unwrap_or_default();
        if let Some((day, set)) = self.built.watermark
            && set <= now
        {
            for adjustment in &last {
                let key = match adjustment {
                    Adjustment::Bill { key, .. } | Adjustment::Short { key, .. } => {
                        Some(key.clone())
                    }
                    // A take-back moves the charge it reconciles, whichever line it comes from:
                    // one due on or after the watermark may take back from any line.
                    Adjustment::Credit { line, .. } => self
                        .lines
                        .iter()
                        .position(|l| l.id == *line)
                        .and_then(|i| owners[i])
                        .map(|owner| expected[owner].key.clone()),
                };
                if let Some(key) = key.filter(|key| self.behind(key, day)) {
                    return Err(format!(
                        "R-094 at {now}: {} moved a charge of {}..{}, behind the watermark {day}",
                        self.describe(adjustment),
                        key.from,
                        key.until
                    ));
                }
            }
        }
        let (_, _, again, _) = self.plan(now)?;
        let again: Vec<String> = again
            .iter()
            .filter(|a| !matches!(a, Adjustment::Short { .. }))
            .map(|a| self.describe(a))
            .collect();
        if !again.is_empty() {
            return Err(format!(
                "idempotency at {now}: a second pass issues {again:?}"
            ));
        }
        Ok(())
    }
}

/// What each lineage's live lines bill, after their notes.
fn billed_by_lineage(lines: &[IssuedLine]) -> BTreeMap<Fee, i64> {
    let mut by: BTreeMap<Fee, i64> = BTreeMap::new();
    for line in lines.iter().filter(|line| !line.voided) {
        *by.entry(line.key.fee.clone()).or_default() += net(line);
    }
    by
}

fn compare(
    what: &str,
    actual: &BTreeMap<Fee, i64>,
    reference: &BTreeMap<Fee, i64>,
    names: &BTreeMap<Fee, String>,
) -> Option<String> {
    let mut fees: Vec<&Fee> = actual.keys().chain(reference.keys()).collect();
    fees.sort();
    fees.dedup();
    let wrong: Vec<String> = fees
        .into_iter()
        .filter(|fee| !matches!(fee, Fee::Minimum { .. }))
        .filter_map(|fee| {
            let (a, r) = (
                actual.get(fee).copied().unwrap_or(0),
                reference.get(fee).copied().unwrap_or(0),
            );
            (a != r).then(|| {
                format!(
                    "{}: {what} {a}, the reference {r}",
                    names
                        .get(fee)
                        .cloned()
                        .unwrap_or_else(|| format!("{fee:?}"))
                )
            })
        })
        .collect();
    (!wrong.is_empty()).then(|| wrong.join("; "))
}

fn run_story(story: &Story) -> Result<(), String> {
    let built = build(story);
    let mut sim = Sim::new(&built);
    let (evaluation, _) = match sim.run() {
        Ok(outcome) => outcome,
        Err(failure) => return Err(sim.report(&failure)),
    };
    let last = built.last();
    let reference = Reference::new(last).totals(&sim.events);
    let packs = reference::packs_billed(&built);
    if let Some(short) = sim.steps.last().and_then(|step| {
        step.adjustments
            .iter()
            .find(|a| matches!(a, Adjustment::Short { .. }))
    }) {
        return Err(sim.report(&format!(
            "R-080: the last pass owes back beyond what lines have left: {}",
            sim.describe(short)
        )));
    }
    // Over its life a subscription bills, net of what was taken back, what its facts owe; a pack,
    // never given back, at least that and at most what each charge was ever owed (R-103).
    if !sim.voided {
        let billed = billed_by_lineage(&sim.lines);
        let mut exact = billed.clone();
        let mut wrong: Vec<String> = Vec::new();
        for (fee, most) in &packs {
            let at = billed.get(fee).copied().unwrap_or(0);
            exact.remove(fee);
            let least = reference.get(fee).copied().unwrap_or(0);
            if at < least || at > *most {
                wrong.push(format!(
                    "{}: billed {at}, the reference between {least} and {most}",
                    sim.name(fee)
                ));
            }
        }
        let mut expected = reference.clone();
        expected.retain(|fee, _| !packs.contains_key(fee));
        wrong.extend(compare("billed", &exact, &expected, &built.names));
        if !wrong.is_empty() {
            return Err(sim.report(&format!("R-079/R-080 lifetime: {}", wrong.join("; "))));
        }
    }
    // What the facts owe in the end is the reference's, whatever was voided on the way.
    if built.watermark.is_none() {
        let mut owed: BTreeMap<Fee, i64> = BTreeMap::new();
        for charge in evaluation.expected() {
            *owed.entry(charge.key.fee).or_default() += charge.amount;
        }
        if let Some(wrong) = compare("the facts owe", &owed, &reference, &built.names) {
            return Err(sim.report(&format!("R-079 reference: {wrong}")));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    /// Rules: R-079, R-080, R-094, R-096, R-002, R-016, R-075, R-084, R-093
    #[test]
    fn a_story_bills_what_its_facts_owe_once(story in story()) {
        if let Err(failure) = run_story(&story) {
            prop_assert!(false, "{}", failure);
        }
    }

    /// Coupons cut no charge and a minimum's commitment is no money owed (R-022): the facts owe
    /// the same without them.
    #[test]
    fn coupons_and_a_minimum_change_no_charge(story in story()) {
        let built = build(&story);
        let facts = built.last().clone();
        let mut bare = facts.clone();
        bare.coupons.clear();
        bare.minimums.clear();
        let events = Events::new(midnight(facts.start.date), built.until + Duration::days(2));
        let now = built.until;
        let reads = |facts: &Facts<Price>| {
            let first = evaluate(facts, now, None, &Book).expect("an evaluation");
            events.reads(&first.usage_needed, now)
        };
        let with = evaluate(&facts, now, Some(&reads(&facts)), &Book).expect("an evaluation");
        let without = evaluate(&bare, now, Some(&reads(&bare)), &Book).expect("an evaluation");
        prop_assert_eq!(with.expected(), without.expected(), "seed {:#x}", seed());
    }
}

#[test]
fn the_reference_bills_a_cancelled_month_by_its_days() {
    use crate::facts::{Billing, Ending, EndingKind, FeeFact, PlanWindow, Start};
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).expect("a day");
    let facts = Facts {
        subscription: story::id(1),
        billing_day_anchor: 1,
        start: Start {
            date: start,
            settled: Some(midnight(start)),
            checkout: false,
        },
        trial: None,
        plan_windows: vec![PlanWindow {
            plan_version: story::id(100),
            from: start,
            to: None,
            cadence: 1,
            cadence_origin: start,
        }],
        fees: vec![FeeFact {
            id: uuid::Uuid::from_u128(10),
            lineage: Fee::Component(story::id(11)),
            from: start,
            to: None,
            billing: Billing::Advance,
            cadence: 1,
            plan_component: true,
            fee: Price {
                kind: Kind::Fixed,
                amount: 3_100,
            },
        }],
        seats: Vec::new(),
        coupons: Vec::new(),
        minimums: Vec::new(),
        ending: Some(Ending {
            at: midnight(NaiveDate::from_ymd_opt(2024, 2, 20).expect("a day")),
            kind: EndingKind::Cancel,
        }),
        end_date: None,
        reconcile_from: None,
        arrears_from: None,
        ending_recorded: None,
        ending_day_invoiced: false,
    };
    let events = Events::new(midnight(start), midnight(start) + Duration::days(90));
    let totals = Reference::new(&facts).totals(&events);
    // January whole, then February's 19 of 29 days: 3100 − round(3100 × 10/29) = 3100 − 1069.
    assert_eq!(
        totals.get(&Fee::Component(story::id(11))),
        Some(&(3_100 + 2_031))
    );
}

/// What `story` bills over its life by fee name, failing on any check `run_story` makes.
fn lifetime(story: &Story) -> (BTreeMap<String, i64>, Vec<(NaiveDate, NaiveDate)>) {
    if let Err(failure) = run_story(story) {
        panic!("{failure}");
    }
    let built = build(story);
    let mut sim = Sim::new(&built);
    sim.run().expect("a run");
    let billed = billed_by_lineage(&sim.lines)
        .into_iter()
        .map(|(fee, amount)| (sim.name(&fee), amount))
        .collect();
    (
        billed,
        sim.lines.iter().map(|l| (l.from, l.until)).collect(),
    )
}

fn day(month: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2023, month, d).expect("a day")
}

fn fixed(price: i64, arrears: bool, cadence: Option<u32>) -> story::FeeSpec {
    story::FeeSpec {
        kind: Kind::Fixed,
        arrears,
        cadence,
        price,
    }
}

fn setup(cadence: u32, fees: Vec<story::FeeSpec>, watermark: Option<u8>) -> story::Setup {
    story::Setup {
        start: 0,
        anchor: 1,
        cadence,
        trial: None,
        fees,
        seats: 0,
        minimum: None,
        coupons: 0,
        watermark,
        unused_first: false,
    }
}

fn plan_change(when: (u8, u8), hour: u8, cadence: u32, keep: bool, percent: u8) -> story::Op {
    story::Op::PlanChange {
        when: story::When {
            periods: when.0,
            day: when.1,
        },
        decided: story::Decided::Same(hour),
        cadence,
        keep_lineage: keep,
        percent,
        setup_fee: None,
    }
}

fn cancel(when: (u8, u8), how: story::How) -> story::End {
    story::End::Cancel {
        when: story::When {
            periods: when.0,
            day: when.1,
        },
        decided: story::Decided::Same(0),
        how,
    }
}

/// 1.00 a quarter in arrears, monthly from Feb 1 by a change on Jan 2, removed on Jan 17, cancelled
/// on Jan 18: the stub's line is dated over its charge's days, Jan 2 to 17, and counts for the
/// month's charge (1), not for the quarter's starting Jan 1 (0) (R-080).
#[test]
fn a_line_is_dated_over_the_charges_its_document_carries() {
    let story = Story {
        setup: setup(3, vec![fixed(1, true, None)], None),
        ops: vec![
            plan_change((0, 1), 0, 1, true, 100),
            story::Op::Remove {
                fee: 0,
                when: story::When {
                    periods: 0,
                    day: 16,
                },
                decided: story::Decided::Same(0),
            },
        ],
        end: cancel((0, 17), story::How::Immediate),
    };
    let (billed, dated) = lifetime(&story);
    assert_eq!(billed, BTreeMap::from([("c1001 Fixed".to_owned(), 1)]));
    assert_eq!(dated, vec![(day(1, 2), day(1, 17))]);
}

/// 0.90 a month, a year at 0.92 from 01:00 on Jan 1, a month at 0.46 from Jan 3, watermark Feb 1:
/// Jan 1's line, taken down to the year's 1, settled that charge, due before the watermark, which
/// bills it no second time (R-094): 1, and 43 for January's stub.
#[test]
fn a_charge_due_before_the_watermark_stays_settled_by_the_line_that_billed_it() {
    let story = Story {
        setup: setup(1, vec![fixed(90, false, None)], Some(2)),
        ops: vec![
            plan_change((0, 0), 1, 12, true, 103),
            plan_change((0, 2), 0, 1, true, 50),
            plan_change((0, 31), 0, 1, false, 50),
        ],
        end: cancel((0, 1), story::How::Immediate),
    };
    let (billed, _) = lifetime(&story);
    assert_eq!(billed.get("c1001 Fixed"), Some(&44));
}

/// 365.00 a year from Jan 1, the watermark at Feb 1, lowered that day to 255.50 (10020 back from
/// Jan 1's line) and raised at 01:00 to 293.82: the raise is due after the watermark, and the
/// note dated on it counts there, so it bills 3832 × 334/365 = 3507 (R-094, R-079).
#[test]
fn a_note_dated_from_the_watermark_counts_against_what_falls_due_from_it() {
    let story = Story {
        setup: setup(
            1,
            vec![fixed(36_500, false, Some(12)), fixed(0, false, None)],
            Some(1),
        ),
        ops: vec![
            plan_change((1, 0), 0, 1, true, 70),
            plan_change((0, 0), 1, 1, true, 115),
        ],
        end: cancel((11, 0), story::How::Dated),
    };
    let (billed, _) = lifetime(&story);
    assert_eq!(billed.get("c1001 Fixed"), Some(&(36_500 - 10_020 + 3_507)));
}

/// A 0.02 pack a month from Jan 1, a 0.01 pack a quarter from 01:00, a 0.01 pack a month from
/// Jan 2: reconcile never gives Jan 1's 2 back, and Jan 2's pack bills its own stub, 1, never
/// netted against it (R-103): 3.
#[test]
fn a_pack_starting_inside_another_packs_period_bills_its_own_price() {
    let story = Story {
        setup: setup(
            1,
            vec![story::FeeSpec {
                kind: Kind::Pack,
                arrears: false,
                cadence: None,
                price: 2,
            }],
            None,
        ),
        ops: vec![
            plan_change((0, 0), 1, 3, true, 50),
            story::Op::PlanChange {
                when: story::When { periods: 0, day: 1 },
                decided: story::Decided::Ahead(1),
                cadence: 1,
                keep_lineage: true,
                percent: 100,
                setup_fee: None,
            },
        ],
        end: cancel((0, 0), story::How::PeriodEnd),
    };
    let (billed, _) = lifetime(&story);
    assert_eq!(billed, BTreeMap::from([("c1001 Pack".to_owned(), 3)]));
    let built = build(&story);
    let bound: Vec<i64> = reference::packs_billed(&built).into_values().collect();
    assert_eq!(bound, vec![3]);
}
