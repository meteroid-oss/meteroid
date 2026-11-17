//! The lifecycle model in the harness, checked after each step against what the previews said,
//! the period the model places, and, once ended, what the reference owes over the life.
mod engine;
mod generator;
mod pinned;
mod reference;

use engine::Engine;
use generator::Op;
use reference::{Action, Catalog, Model, PlanSpec, Refused, plan_name};

use super::harness::{self, Diff, Knobs, ModelHarness};

fn when(at: reference::At) -> String {
    at.format("%Y-%m-%d %H:%M").to_string()
}

fn money(cents: i64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

fn plan(spec: &PlanSpec) -> String {
    format!(
        "{} {} every {} month(s), {} a unit, {} a seat",
        money(spec.fee),
        if spec.arrears {
            "in arrears"
        } else {
            "in advance"
        },
        spec.months,
        money(spec.rate),
        money(spec.seat)
    )
}

fn catalog(catalog: &Catalog) -> String {
    format!(
        "A = {}; B = {}; {} seat(s); add-on {} every {} month(s){}; documents {}",
        plan(&catalog.plans[0]),
        plan(&catalog.plans[1]),
        catalog.seats,
        money(catalog.add_on.price),
        catalog.add_on.months,
        if catalog.add_on.on_b {
            " on both plans"
        } else {
            " on A"
        },
        if catalog.auto_advance {
            "finalize on their own"
        } else {
            "stay drafts until finalized"
        }
    ) + &extras(catalog)
}

fn extras(catalog: &Catalog) -> String {
    let mut found = String::new();
    if let Some(coupon) = catalog.coupon {
        let off = match coupon.off {
            reference::Off::Percent(p) => format!("{p}%"),
            reference::Off::Fixed(cents) => money(cents),
        };
        let times = coupon.invoices.map_or("on every invoice".to_owned(), |n| {
            format!("on {n} invoice(s)")
        });
        let expires = coupon
            .expires
            .map(|day| format!(", redeemable to {day}"))
            .unwrap_or_default();
        found += &format!("; coupon {off} off {times}{expires}");
    }
    if let Some(minimum) = catalog.minimum {
        found += &format!("; minimum {} a period", money(minimum));
    }
    if let Some(trial) = catalog.trial {
        found += &format!(
            "; free trial to {} {}{}",
            trial.end,
            if trial.card {
                "with a card"
            } else {
                "without a card"
            },
            trial
                .anchor
                .map(|day| format!(", billed on day {day}"))
                .unwrap_or_default()
        );
    }
    if let Some(end) = catalog.term_end {
        found += &format!("; fixed term to {end}");
    }
    if let Some(capacity) = catalog.capacity {
        found += &format!(
            "; A's capacity {} for {} units, {} a unit past them",
            money(capacity.fee),
            capacity.included,
            money(capacity.overage)
        );
    }
    if let Some(fee) = catalog.companion {
        found += &format!("; a companion at {} a month, consolidated", money(fee));
    }
    found
}

impl harness::Reference for Model {
    type Op = Op;
    type Action = Action;
    type Refusal = Refused;

    fn generate(seed: u64, steps: usize) -> Vec<Op> {
        generator::generate(seed, steps)
    }

    fn resolve(&self, op: &Op) -> Option<Action> {
        generator::resolve(self, op)
    }

    fn kind(action: &Action) -> &'static str {
        match action {
            Action::Start { .. } => "Start",
            Action::Usage { .. } => "Usage",
            Action::Advance { .. } => "Advance",
            Action::AmendNow { .. } => "AmendNow",
            Action::AmendAtEnd { .. } => "AmendAtEnd",
            Action::ChangeNow { .. } => "ChangeNow",
            Action::ChangeAtEnd { .. } => "ChangeAtEnd",
            Action::OneTime { .. } => "OneTime",
            Action::CancelNow { .. } => "CancelNow",
            Action::CancelImmediately { .. } => "CancelImmediately",
            Action::CancelBackdated { .. } => "CancelBackdated",
            Action::BuySeats { .. } => "BuySeats",
            Action::RemoveSeats { .. } => "RemoveSeats",
            Action::AttachAddOn { .. } => "AttachAddOn",
            Action::AddOnQuantity { .. } => "AddOnQuantity",
            Action::DetachAddOn { .. } => "DetachAddOn",
            Action::CancelAtEnd { .. } => "CancelAtEnd",
            Action::Void { .. } => "Void",
            Action::Pay { .. } => "Pay",
            Action::Finalize { .. } => "Finalize",
            Action::FinalizeAll { .. } => "FinalizeAll",
        }
    }

    fn describe(&self, action: &Action) -> String {
        match action {
            Action::Start { day, catalog: c } => {
                format!(
                    "{}  start on A: {}",
                    when(reference::midnight(*day)),
                    catalog(c)
                )
            }
            Action::Usage { at, units } => format!("{}  usage: {units} units", when(*at)),
            Action::Advance { at } => {
                let sub = self.sub();
                let what = match sub.scheduled {
                    Some(reference::Scheduled::Cancel(on)) if on <= at.date_naive() => {
                        "the period ends, final invoice"
                    }
                    _ => "renew",
                };
                format!("{}  {what} (plan {})", when(*at), plan_name(sub.plan))
            }
            Action::AmendNow {
                at, price, months, ..
            } => format!(
                "{}  amend the fee to {} every {months} month(s), now",
                when(*at),
                money(*price)
            ),
            Action::AmendAtEnd { at, price, .. } => format!(
                "{}  amend the fee to {} at the period end",
                when(*at),
                money(*price)
            ),
            Action::ChangeNow { at, to } => {
                format!("{}  change to plan {} now", when(*at), plan_name(*to))
            }
            Action::ChangeAtEnd { at, to, seats } => format!(
                "{}  change to plan {} at the period end{}",
                when(*at),
                plan_name(*to),
                seats
                    .map(|n| format!(", its seats set to {n}"))
                    .unwrap_or_default()
            ),
            Action::OneTime { at, price } => {
                format!("{}  add a one-time fee of {}", when(*at), money(*price))
            }
            Action::CancelNow { at } => format!("{}  cancel from today", when(*at)),
            Action::CancelImmediately { at } => format!("{}  cancel now", when(*at)),
            Action::CancelBackdated { at, day } => {
                format!("{}  cancel from {day}, backdated", when(*at))
            }
            Action::BuySeats { at, seats } => format!("{}  buy {seats} seat(s)", when(*at)),
            Action::RemoveSeats { at, seats } => {
                format!("{}  remove {seats} seat(s) at the period end", when(*at))
            }
            Action::AttachAddOn { at, quantity } => {
                format!("{}  attach the add-on x{quantity}", when(*at))
            }
            Action::AddOnQuantity { at, quantity } => {
                format!("{}  set the add-on to x{quantity}", when(*at))
            }
            Action::DetachAddOn { at } => format!("{}  detach the add-on", when(*at)),
            Action::CancelAtEnd { at, end } => {
                format!("{}  cancel at the period end ({end})", when(*at))
            }
            Action::Void { at, pick } => {
                format!("{}  void a finalized invoice ({pick})", when(*at))
            }
            Action::Pay { at, pick } => format!("{}  pay an invoice owing ({pick})", when(*at)),
            Action::Finalize { at, pick } => format!("{}  finalize a draft ({pick})", when(*at)),
            Action::FinalizeAll { at } => format!("{}  finalize every draft", when(*at)),
        }
    }

    fn apply(&mut self, action: &Action) -> Result<(), Refused> {
        Model::apply(self, action)
    }

    fn reason(refusal: &Refused) -> &str {
        refusal.0
    }

    fn state(&self) -> String {
        let sub = self.sub();
        format!(
            "        expected: plan {} period {}..{}, {} seat(s){}{}{}{}",
            plan_name(sub.plan),
            sub.period_start,
            sub.period_end,
            sub.seat_count,
            if sub.seats_off() > 0 {
                format!(" ({} off at the end)", sub.seats_off())
            } else {
                String::new()
            },
            sub.add_on
                .map(|(_, quantity)| format!(", add-on x{quantity}"))
                .unwrap_or_default(),
            sub.scheduled
                .map(|s| format!(", {s:?} pending"))
                .unwrap_or_default(),
            self.owed()
                .map(|owed| format!(", owes {} over its life", money(owed)))
                .unwrap_or_default()
        )
    }
}

impl harness::Engine<Model> for Engine {
    type Fingerprint = Vec<String>;

    async fn new() -> Self {
        Engine::new().await
    }

    async fn before(&mut self, _: &mut Model, _: &Action) -> Vec<Diff> {
        vec![]
    }

    async fn apply(&mut self, action: &Action, model: &Model) -> Result<(), String> {
        Engine::apply(self, action, model).await
    }

    async fn compare(&self, model: &Model) -> Vec<Diff> {
        Engine::compare(self, model).await
    }

    async fn invariants(&self) -> Vec<String> {
        Engine::invariants(self).await
    }

    async fn fingerprint(&self) -> Vec<String> {
        Engine::fingerprint(self).await
    }

    async fn rerun_workers(&self, action: &Action) -> Result<(), String> {
        Engine::rerun_workers(self, action).await
    }

    async fn dump(&self) -> Vec<String> {
        Engine::dump(self).await
    }
}

/// LIFECYCLE_MODEL_CASES cases from LIFECYCLE_MODEL_SEED, LIFECYCLE_MODEL_STEPS operations each;
/// one at a time, as each moves the process's clock; the first LIFECYCLE_MODEL_SHRINK distinct
/// failures are shrunk.
pub async fn run_cases(shard: u64, shards: u64) {
    ModelHarness::<Model, Engine>::run_cases(Knobs {
        prefix: "LIFECYCLE_MODEL",
        label: "lifecycle model",
        cases: 8,
        seed: 0x11FE_2026,
        steps: 24,
        jobs: 1,
        moves_clock: true,
        shard,
        shards,
    })
    .await;
}
