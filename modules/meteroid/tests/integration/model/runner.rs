//! The credit model in the harness: how its actions read, and how the engine's documents and
//! grants compare with the model's after every step.
use std::collections::BTreeMap;

use super::engine::{Engine, Fingerprint, sub_name, token_name};
use super::generator::{Op, generate, resolve};
use super::harness::{self, Diff, Knobs, ModelHarness};
use super::reference::{Action, At, How, Kind, Late, Model, NoteType, Plan, Refused};

fn when(at: At) -> String {
    at.format("%Y-%m-%d %H:%M").to_string()
}

fn plan(plan: Plan) -> &'static str {
    match plan {
        Plan::Payg => "Pay as you go",
        Plan::Scale => "Scale",
    }
}

fn kind_of(action: &Action) -> &'static str {
    match action {
        Action::Start { .. } => "Start",
        Action::Usage { at, dated, .. } if at != dated => "LateUsage",
        Action::Usage { .. } => "Usage",
        Action::Renew { .. } => "Renew",
        Action::TopUp { .. } => "TopUp",
        Action::Pay { .. } => "Pay",
        Action::Fail { .. } => "Fail",
        Action::Chargeback { .. } => "Chargeback",
        Action::RefundNote { .. } => "RefundNote",
        Action::RefundNoNote { .. } => "RefundNoNote",
        Action::Void { .. } => "Void",
        Action::Redeem { .. } => "Redeem",
        Action::ChangeNow { .. } => "ChangeNow",
        Action::ChangeAtEnd { .. } => "ChangeAtEnd",
        Action::CancelNow { .. } => "CancelNow",
        Action::CancelAtEnd { .. } => "CancelAtEnd",
    }
}

pub fn describe(action: &Action, model: &Model) -> String {
    let inv = |i: &usize| format!("INV-{}", i + 1);
    let late = |late: &Option<super::reference::LateUse>| match late {
        None => String::new(),
        Some(l) => format!(
            ", then {}M {} ({}) dated an hour before, ingested {}",
            l.mtok,
            token_name(l.token),
            l.token.rate() * l.mtok,
            match l.when {
                Late::WithinGrace => "within the grace",
                Late::AfterGrace => "after the grace",
            }
        ),
    };
    match action {
        Action::Start { day, plan: p } => format!(
            "{}  start {} on {}",
            when(super::reference::midnight(*day)),
            sub_name(model.subs.len()),
            plan(*p)
        ),
        Action::Usage {
            sub,
            at,
            dated,
            token,
            mtok,
        } => {
            let amount = token.rate() * mtok;
            if at == dated {
                format!(
                    "{}  usage {}: {mtok}M {} = {amount}",
                    when(*at),
                    sub_name(*sub),
                    token_name(*token)
                )
            } else {
                format!(
                    "{}  late usage {} dated {}: {mtok}M {} = {amount}",
                    when(*at),
                    sub_name(*sub),
                    when(*dated),
                    token_name(*token)
                )
            }
        }
        Action::Renew { at, subs } => format!(
            "{}  renew {}",
            when(*at),
            subs.iter()
                .map(|s| sub_name(*s))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Action::TopUp { at, amount, pay } => format!(
            "{}  top-up {amount} (INV-{}){}",
            when(*at),
            model.invoices.len() + 1,
            if *pay { ", paid at once" } else { ", unpaid" }
        ),
        Action::Pay { at, invoice, how } => format!(
            "{}  pay {} ({} due) {}",
            when(*at),
            inv(invoice),
            model.invoices[*invoice].due(),
            match how {
                How::Manual => "by hand".to_owned(),
                How::Provider(0) => "through the provider".to_owned(),
                How::Provider(extra) => format!("through the provider, {extra} above what is due"),
            }
        ),
        Action::Fail { at, invoice } => {
            format!(
                "{}  a provider payment of {} fails",
                when(*at),
                inv(invoice)
            )
        }
        Action::Chargeback {
            at,
            invoice,
            payment,
            disputed,
        } => format!(
            "{}  {} of payment {} of {} ({})",
            when(*at),
            if *disputed {
                "chargeback"
            } else {
                "late failure"
            },
            payment + 1,
            inv(invoice),
            model.invoices[*invoice].payments[*payment].amount
        ),
        Action::RefundNote {
            at,
            invoice,
            amount,
            ..
        } => format!("{}  refund note of {amount} on {}", when(*at), inv(invoice)),
        Action::RefundNoNote {
            at,
            invoice,
            amount,
            ..
        } => format!(
            "{}  refund {amount} of {} without a credit note",
            when(*at),
            inv(invoice)
        ),
        Action::Void { at, invoice } => format!("{}  void {}", when(*at), inv(invoice)),
        Action::Redeem { at } => format!("{}  redeem the promotion code", when(*at)),
        Action::ChangeNow {
            sub,
            day,
            to,
            late: l,
        } => format!(
            "{}  change {} to {} now{}",
            when(super::reference::midnight(*day)),
            sub_name(*sub),
            plan(*to),
            late(l)
        ),
        Action::ChangeAtEnd { at, sub, to } => format!(
            "{}  change {} to {} at the period end",
            when(*at),
            sub_name(*sub),
            plan(*to)
        ),
        Action::CancelNow { sub, day, late: l } => format!(
            "{}  cancel {} now{}",
            when(super::reference::midnight(*day)),
            sub_name(*sub),
            late(l)
        ),
        Action::CancelAtEnd { at, sub, end } => format!(
            "{}  cancel {} at the period end ({end})",
            when(*at),
            sub_name(*sub)
        ),
    }
}

fn grant_key(model: &Model, g: usize) -> String {
    let grant = &model.grants[g];
    let kind = match grant.kind {
        Kind::Pack => "Pack",
        Kind::Bonus => "Bonus",
        Kind::Welcome => "Welcome",
        Kind::TopUp => "TopUp",
        Kind::Promo => "Promo",
        Kind::Note => "Note",
        Kind::Overpay => "Overpay",
    };
    match grant.sub {
        Some(s) => format!("{kind}/{}", sub_name(s)),
        None => kind.to_owned(),
    }
}

fn expected_grants(model: &Model) -> BTreeMap<String, i64> {
    let mut grants = BTreeMap::new();
    for g in 0..model.grants.len() {
        if model.grants[g].remaining != 0 {
            *grants.entry(grant_key(model, g)).or_insert(0) += model.grants[g].remaining;
        }
    }
    grants
}

const FIGURES: [&str; 7] = [
    "subtotal", "discount", "promo", "tax", "total", "applied", "due",
];

/// Each difference as (category, detail); the category names the kind of mismatch.
fn compare(model: &Model, observed: &super::engine::Observed) -> Vec<Diff> {
    let mut diffs = vec![];
    if observed.invoices.len() != model.invoices.len() || !observed.unmapped.is_empty() {
        diffs.push((
            "invoice count".to_owned(),
            format!(
                "expected {} invoices, the engine issued {} (unmapped: {:?})",
                model.invoices.len(),
                observed.invoices.len(),
                observed.unmapped
            ),
        ));
    }
    for (i, (inv, (status, figures))) in model.invoices.iter().zip(&observed.invoices).enumerate() {
        let expected = [
            inv.subtotal,
            0,
            inv.promo,
            0,
            inv.total,
            inv.applied,
            inv.due(),
        ];
        let void = *status == meteroid_store::domain::enums::InvoiceStatusEnum::Void;
        if void != inv.void {
            diffs.push((
                "invoice status".to_owned(),
                format!(
                    "INV-{}: expected void {}, engine {status:?}",
                    i + 1,
                    inv.void
                ),
            ));
        }
        for (k, name) in FIGURES.iter().enumerate() {
            if (name == &"due" && inv.void) || expected[k] == figures[k] {
                continue;
            }
            diffs.push((
                format!("invoice {name}"),
                format!(
                    "INV-{}: {name} expected {}, engine {} (expected {:?}, engine {:?})",
                    i + 1,
                    expected[k],
                    figures[k],
                    expected,
                    figures
                ),
            ));
            break;
        }
        let mut notes: Vec<(String, i64)> = inv
            .notes
            .iter()
            .map(|(kind, total)| {
                let kind = match kind {
                    NoteType::Balance => "Balance",
                    NoteType::Refund => "Refund",
                };
                (kind.to_owned(), *total)
            })
            .collect();
        notes.sort();
        if let Some(engine) = observed.notes.get(i)
            && *engine != notes
        {
            diffs.push((
                "credit notes".to_owned(),
                format!("INV-{}: notes expected {notes:?}, engine {engine:?}", i + 1),
            ));
        }
    }
    let grants = expected_grants(model);
    if grants != observed.grants {
        let keys: std::collections::BTreeSet<&String> =
            grants.keys().chain(observed.grants.keys()).collect();
        for key in keys {
            let (e, a) = (
                grants.get(key).copied().unwrap_or(0),
                observed.grants.get(key).copied().unwrap_or(0),
            );
            if e != a {
                diffs.push((
                    format!("grant {}", key.split('/').next().unwrap_or(key)),
                    format!("{key} remaining expected {e}, engine {a}"),
                ));
            }
        }
    }
    diffs
}

fn state(model: &Model) -> String {
    let invoices: Vec<String> = model
        .invoices
        .iter()
        .enumerate()
        .map(|(i, inv)| {
            format!(
                "INV-{} {}{}/{}/{}/{}/{}",
                i + 1,
                if inv.void { "void " } else { "" },
                inv.subtotal,
                inv.promo,
                inv.total,
                inv.applied,
                inv.due()
            )
        })
        .collect();
    format!(
        "        expected: {} | credit {:?}",
        invoices.join(", "),
        expected_grants(model)
    )
}

impl harness::Reference for Model {
    type Op = Op;
    type Action = Action;
    type Refusal = Refused;

    fn generate(seed: u64, steps: usize) -> Vec<Op> {
        generate(seed, steps)
    }

    fn resolve(&self, op: &Op) -> Option<Action> {
        resolve(self, op)
    }

    fn kind(action: &Action) -> &'static str {
        kind_of(action)
    }

    fn describe(&self, action: &Action) -> String {
        describe(action, self)
    }

    fn apply(&mut self, action: &Action) -> Result<(), Refused> {
        Model::apply(self, action)
    }

    fn reason(refusal: &Refused) -> &str {
        refusal.0
    }

    fn state(&self) -> String {
        state(self)
    }
}

impl harness::Engine<Model> for Engine {
    type Fingerprint = Fingerprint;

    async fn new() -> Self {
        Engine::new().await
    }

    /// The expiry sweeps due before the action, one per lapse, on both sides.
    async fn before(&mut self, model: &mut Model, action: &Action) -> Vec<Diff> {
        let mut details = vec![];
        if model.now.is_some() {
            for at in model.sweeps_due(action.at()) {
                model.sweep(at);
                if let Err(e) = self.sweep(at).await {
                    details.push(("engine error".to_owned(), format!("sweep at {at}: {e}")));
                }
            }
        }
        details
    }

    async fn apply(&mut self, action: &Action, model: &Model) -> Result<(), String> {
        Engine::apply(self, action, model).await
    }

    async fn compare(&self, model: &Model) -> Vec<Diff> {
        compare(model, &self.observe().await)
    }

    async fn invariants(&self) -> Vec<String> {
        Engine::invariants(self).await
    }

    async fn fingerprint(&self) -> Fingerprint {
        Engine::fingerprint(self).await
    }

    async fn rerun_workers(&self, action: &Action) -> Result<(), String> {
        Engine::rerun_workers(self, action).await
    }

    async fn dump(&self) -> Vec<String> {
        Engine::dump(self)
            .await
            .into_iter()
            .map(|line| format!("ledger: {line}"))
            .collect()
    }
}

/// The `shard`-th of `shards` slices of MODEL_CASES cases from MODEL_SEED, MODEL_STEPS operations
/// each, MODEL_JOBS at a time; the first MODEL_SHRINK distinct failures are shrunk and printed.
pub async fn run_cases(shard: u64, shards: u64) {
    ModelHarness::<Model, Engine>::run_cases(Knobs {
        prefix: "MODEL",
        label: "model",
        cases: 16,
        seed: 0x00C0_FFEE,
        steps: 25,
        shard,
        shards,
    })
    .await;
}
