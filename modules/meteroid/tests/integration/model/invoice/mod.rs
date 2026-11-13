//! The invoice model in the harness: how its actions and expected documents read.
mod engine;
mod generator;
mod reference;
mod regressions;

use engine::Engine;
use generator::Op;
use reference::{Action, Billing, Catalog, Discount, Model, PlanSpec, Refused, plan_name};

use super::harness::{self, Diff, Knobs, ModelHarness};

fn when(at: reference::At) -> String {
    at.format("%Y-%m-%d %H:%M").to_string()
}

fn catalog(catalog: &Catalog) -> String {
    let money = |minor| catalog.currency.format(minor);
    let plan = |p: usize, spec: &PlanSpec| {
        format!(
            "{} = fee {} {}, {} per unit",
            plan_name(p),
            money(spec.fee),
            match spec.billing {
                Billing::Advance => "in advance",
                Billing::Arrears => "in arrears",
            },
            rust_decimal::Decimal::new(
                spec.rate,
                catalog.currency.exponent() + reference::RATE_SCALE.ilog10()
            )
            .normalize()
        )
    };
    let coupons: Vec<String> = catalog
        .coupons
        .iter()
        .map(|c| {
            format!(
                "{} for {}",
                match c.discount {
                    Discount::Percent(p) => format!("{p}%"),
                    Discount::Fixed(amount) => format!("{} off", money(amount)),
                },
                c.cycles
                    .map_or("every invoice".to_owned(), |n| format!("{n} invoice(s)"))
            )
        })
        .collect();
    let taxes: Vec<String> = catalog
        .taxes
        .iter()
        .map(|t| {
            format!(
                "{} {}%",
                t.name,
                rust_decimal::Decimal::new(t.ppm, 4).normalize()
            )
        })
        .collect();
    format!(
        "{}; {}; {}; coupons [{}]; taxes [{}]",
        catalog.currency.code(),
        plan(0, &catalog.plans[0]),
        plan(1, &catalog.plans[1]),
        coupons.join(", "),
        taxes.join(", ")
    )
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
            Action::ChangeAtEnd { .. } => "ChangeAtEnd",
            Action::CancelAtEnd { .. } => "CancelAtEnd",
        }
    }

    fn describe(&self, action: &Action) -> String {
        match action {
            Action::Start { day, catalog: c } => format!(
                "{}  start on A: {}",
                when(reference::midnight(*day)),
                catalog(c)
            ),
            Action::Usage { at, units } => format!("{}  usage: {units} units", when(*at)),
            Action::Advance { at } => {
                let sub = self.sub();
                let what = match sub.scheduled {
                    Some(reference::Scheduled::Cancel) => "the period ends, final invoice",
                    _ => "renew",
                };
                format!("{}  {what} (plan {})", when(*at), plan_name(sub.plan))
            }
            Action::ChangeAtEnd { at, to } => format!(
                "{}  change to plan {} at the period end",
                when(*at),
                plan_name(*to)
            ),
            Action::CancelAtEnd { at, end } => {
                format!("{}  cancel at the period end ({end})", when(*at))
            }
        }
    }

    fn apply(&mut self, action: &Action) -> Result<(), Refused> {
        Model::apply(self, action)
    }

    fn reason(refusal: &Refused) -> &str {
        refusal.0
    }

    fn state(&self) -> String {
        let invoices: Vec<String> = self
            .invoices
            .iter()
            .enumerate()
            .map(|(i, inv)| {
                format!(
                    "INV-{} {}/{}/{}/{} {:?}",
                    i + 1,
                    inv.subtotal,
                    inv.discount,
                    inv.tax,
                    inv.total,
                    inv.lines
                )
            })
            .collect();
        format!("        expected: {}", invoices.join(", "))
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

    async fn apply(&mut self, action: &Action, _: &Model) -> Result<(), String> {
        Engine::apply(self, action).await
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

/// INVOICE_MODEL_CASES cases from INVOICE_MODEL_SEED, INVOICE_MODEL_STEPS operations each,
/// INVOICE_MODEL_JOBS at a time; the first INVOICE_MODEL_SHRINK distinct failures are shrunk.
pub async fn run_cases(shard: u64, shards: u64) {
    ModelHarness::<Model, Engine>::run_cases(Knobs {
        prefix: "INVOICE_MODEL",
        label: "invoice model",
        cases: 16,
        seed: 0x1_2025,
        steps: 20,
        shard,
        shards,
    })
    .await;
}
