//! Runs generated sequences against the engine and the model, compares them after every step,
//! shrinks a failing sequence and prints it as a dated scenario.
use std::collections::BTreeMap;
use std::time::Instant;

use futures::StreamExt;

use super::engine::{Engine, sub_name, token_name};
use super::generator::{Op, generate, resolve};
use super::reference::{Action, At, How, Kind, Late, Model, NoteType, Plan};

#[derive(Clone, Debug)]
pub struct Failure {
    pub step: usize,
    pub signature: String,
    pub details: Vec<String>,
}

pub struct Run {
    pub lines: Vec<String>,
    pub kinds: Vec<&'static str>,
    pub failure: Option<Failure>,
}

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
fn compare(model: &Model, observed: &super::engine::Observed) -> Vec<(String, String)> {
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

/// One case: every operation resolved against the model, run on both, then compared.
pub async fn run_case(ops: &[Op]) -> Run {
    let mut engine = Engine::new().await;
    let run = run_with(&mut engine, ops).await;
    // Dropping a test database blocks on DROP DATABASE, which waits for every backend, including
    // another case's connection still authenticating on this task: drop it off the task.
    tokio::task::spawn_blocking(move || drop(engine))
        .await
        .expect("the test database dropped");
    run
}

async fn run_with(engine: &mut Engine, ops: &[Op]) -> Run {
    let mut model = Model::default();
    let mut lines = vec![];
    let mut kinds = vec![];
    for (step, op) in ops.iter().enumerate() {
        let Some(action) = resolve(&model, op) else {
            continue;
        };
        let mut details: Vec<(String, String)> = vec![];
        if model.now.is_some() {
            for at in model.sweeps_due(action.at()) {
                model.sweep(at);
                if let Err(e) = engine.sweep(at).await {
                    details.push(("engine error".to_owned(), format!("sweep at {at}: {e}")));
                }
            }
        }
        lines.push(describe(&action, &model));
        kinds.push(kind_of(&action));
        let before = model.clone();
        let expected = model.apply(&action);
        let actual = engine.apply(&action, &before).await;
        match (&expected, &actual) {
            (Ok(()), Ok(())) | (Err(_), Err(_)) => {}
            (Ok(()), Err(e)) => {
                details.push(("outcome".to_owned(), format!("engine refused: {e}")))
            }
            (Err(r), Ok(())) => details.push((
                "outcome".to_owned(),
                format!("expected a refusal ({}), the engine accepted", r.0),
            )),
        }
        if let (Err(r), Err(e)) = (&expected, &actual) {
            lines.push(format!("        refused as expected ({}): {e}", r.0));
        }
        let observed = engine.observe().await;
        details.extend(compare(&model, &observed));
        for violation in engine.invariants().await {
            let name = violation.split(": ").next().unwrap_or_default().to_owned();
            details.push((format!("invariant {name}"), violation));
        }
        if details.is_empty() {
            let first = engine.fingerprint().await;
            match engine.rerun_workers(&action).await {
                Err(e) => details.push(("idempotency".to_owned(), e)),
                Ok(()) => {
                    let second = engine.fingerprint().await;
                    if first != second {
                        details.push((
                            "idempotency".to_owned(),
                            format!("a second pass changed {first:?} into {second:?}"),
                        ));
                    }
                }
            }
        }
        lines.push(state(&model));
        if !details.is_empty() {
            details.extend(
                engine
                    .dump()
                    .await
                    .into_iter()
                    .map(|line| ("ledger".to_owned(), format!("ledger: {line}"))),
            );
            let signature = format!("{}: {}", kind_of(&action), details[0].0);
            return Run {
                lines,
                kinds,
                failure: Some(Failure {
                    step,
                    signature,
                    details: details.into_iter().map(|(_, d)| d).collect(),
                }),
            };
        }
    }
    Run {
        lines,
        kinds,
        failure: None,
    }
}

/// Truncates after the failing step, then drops every operation whose removal keeps the same
/// failure, last first.
pub async fn shrink(ops: &[Op], failure: &Failure, budget: usize) -> (Vec<Op>, Run) {
    let mut best: Vec<Op> = ops[..=failure.step].to_vec();
    let mut best_run = run_case(&best).await;
    let mut runs = 1;
    let mut i = best.len().saturating_sub(1);
    while i >= 1 && runs < budget {
        let mut candidate = best.clone();
        candidate.remove(i);
        let run = run_case(&candidate).await;
        runs += 1;
        if let Some(f) = &run.failure
            && f.signature == failure.signature
        {
            candidate.truncate(f.step + 1);
            best = candidate;
            best_run = run;
            i = i.min(best.len());
        }
        i -= 1;
    }
    (best, best_run)
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

/// The `shard`-th of `shards` slices of MODEL_CASES cases from MODEL_SEED, MODEL_STEPS operations
/// each, MODEL_JOBS at a time; the first MODEL_SHRINK distinct failures are shrunk and printed.
pub async fn run_cases(shard: u64, shards: u64) {
    let cases = env("MODEL_CASES", 16);
    let base = env("MODEL_SEED", 0x00C0_FFEE);
    let steps = env("MODEL_STEPS", 25) as usize;
    let jobs = env("MODEL_JOBS", 4) as usize;
    let shrinks = env("MODEL_SHRINK", 8) as usize;
    let started = Instant::now();
    let seeds: Vec<u64> = (0..cases)
        .filter(|i| i % shards == shard)
        .map(|i| base.wrapping_add(i))
        .collect();
    let results: Vec<(u64, Vec<Op>, Run)> = futures::stream::iter(seeds)
        .map(|seed| async move {
            let ops = generate(seed, steps);
            let run = run_case(&ops).await;
            (seed, ops, run)
        })
        .buffer_unordered(jobs)
        .collect()
        .await;
    let mut failures: Vec<&(u64, Vec<Op>, Run)> = results
        .iter()
        .filter(|(_, _, r)| r.failure.is_some())
        .collect();
    failures.sort_by_key(|(seed, _, _)| *seed);
    eprintln!(
        "model tests: {} cases from seed {base:#x}, {} failed, {:?}",
        results.len(),
        failures.len(),
        started.elapsed()
    );
    let mut actions: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, _, run) in &results {
        for kind in &run.kinds {
            *actions.entry(kind).or_default() += 1;
        }
    }
    eprintln!("  actions run: {actions:?}");
    let mut by_signature: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for (seed, _, run) in &failures {
        let failure = run.failure.as_ref().expect("failed");
        by_signature
            .entry(failure.signature.clone())
            .or_default()
            .push(*seed);
    }
    for (signature, seeds) in &by_signature {
        eprintln!(
            "  {signature}: {} case(s), seeds {}",
            seeds.len(),
            seeds
                .iter()
                .map(|s| format!("{s:#x}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    for (signature, seeds) in &by_signature {
        let (_, _, run) = failures
            .iter()
            .find(|(seed, _, _)| *seed == seeds[0])
            .expect("the first seed");
        let failure = run.failure.as_ref().expect("failed");
        eprintln!("\n--- {signature}, seed {:#x}, unshrunk ---", seeds[0]);
        for line in &run.lines {
            eprintln!("{line}");
        }
        for detail in &failure.details {
            eprintln!("      {detail}");
        }
    }
    let firsts: Vec<(String, u64, Vec<Op>, Failure)> = by_signature
        .iter()
        .map(|(signature, seeds)| {
            let (_, ops, run) = failures
                .iter()
                .find(|(seed, _, _)| *seed == seeds[0])
                .expect("the first seed");
            let failure = run.failure.clone().expect("failed");
            (signature.clone(), seeds[0], ops.clone(), failure)
        })
        .collect();
    let shrunk: Vec<Vec<String>> = futures::stream::iter(firsts.into_iter().enumerate())
        .map(|(n, (signature, seed, ops, failure))| async move {
            let (lines, failure) = if n < shrinks {
                let (_, run) = shrink(&ops, &failure, 40).await;
                let failure = run.failure.clone().unwrap_or(failure);
                (run.lines, failure)
            } else {
                let run = run_case(&ops[..=failure.step]).await;
                (run.lines, failure)
            };
            let mut report = vec![format!(
                "\n=== {signature} (seed {seed:#x}, replay with MODEL_SEED={seed:#x} MODEL_CASES=1) ==="
            )];
            report.extend(lines);
            report.push("    mismatch:".to_owned());
            report.extend(failure.details.iter().map(|d| format!("      {d}")));
            report
        })
        .buffer_unordered(jobs)
        .collect()
        .await;
    let report: Vec<String> = shrunk.into_iter().flatten().collect();
    eprintln!("{}", report.join("\n"));
    assert!(
        failures.is_empty(),
        "{} of {} model cases failed: {:?}",
        failures.len(),
        results.len(),
        by_signature.keys().collect::<Vec<_>>()
    );
}
