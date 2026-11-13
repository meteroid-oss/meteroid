//! The domain-free part of a model test: seeded cases run against an engine and a reference,
//! compared after every step with the invariants and a second pass of every worker, the failures
//! shrunk and printed with the seed that replays them.
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::time::Instant;

use futures::StreamExt;

/// SplitMix64: deterministic for a seed, no dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn byte(&mut self) -> u8 {
        (self.next() >> 56) as u8
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Stack of every thread a model test runs on. A debug build keeps each child future of an
/// async fn in its own stack slot, and the engine's deepest calls outgrow the default 2 MiB.
const STACK: usize = 32 << 20;

/// Runs `test` as `#[tokio::test(flavor = "multi_thread", worker_threads = 4)]` does, on threads
/// with `STACK` bytes of stack.
pub fn on_large_stacks<F: Future<Output = ()>>(test: impl FnOnce() -> F + Send + 'static) {
    let name = std::thread::current().name().map(str::to_owned);
    let mut thread = std::thread::Builder::new().stack_size(STACK);
    if let Some(name) = name {
        thread = thread.name(name);
    }
    let outcome = thread
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .thread_stack_size(STACK)
                .enable_all()
                .build()
                .expect("the test runtime")
                .block_on(test());
        })
        .expect("the test thread")
        .join();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// A difference as (category, detail); the category names the kind of mismatch.
pub type Diff = (String, String);

/// The model side: operations hold only choices and are resolved against the model's state when
/// run, so removing operations while shrinking keeps a sequence valid.
pub trait Reference: Clone + Default {
    type Op: Clone + Debug;
    type Action: Clone + Debug;
    type Refusal;

    fn generate(seed: u64, steps: usize) -> Vec<Self::Op>;
    fn resolve(&self, op: &Self::Op) -> Option<Self::Action>;
    fn kind(action: &Self::Action) -> &'static str;
    fn describe(&self, action: &Self::Action) -> String;
    fn apply(&mut self, action: &Self::Action) -> Result<(), Self::Refusal>;
    fn reason(refusal: &Self::Refusal) -> &str;
    /// The expected state after a step, printed under it.
    fn state(&self) -> String;
}

/// The engine side, driven through the same actions in business time.
pub trait Engine<R: Reference>: Sized + Send + 'static {
    type Fingerprint: PartialEq + Debug;

    async fn new() -> Self;
    /// Work due on both sides before `action` runs, e.g. sweeps; reports engine errors.
    async fn before(&mut self, model: &mut R, action: &R::Action) -> Vec<Diff>;
    /// Runs `action`; `model` is the state before it.
    async fn apply(&mut self, action: &R::Action, model: &R) -> Result<(), String>;
    async fn compare(&self, model: &R) -> Vec<Diff>;
    async fn invariants(&self) -> Vec<String>;
    /// Everything a second pass of a worker could change.
    async fn fingerprint(&self) -> Self::Fingerprint;
    /// Every sweep and worker a second time, at the same business instant: none may write.
    async fn rerun_workers(&self, action: &R::Action) -> Result<(), String>;
    /// What the engine stored, for a failure report.
    async fn dump(&self) -> Vec<String>;
}

/// Sizes a run from `{prefix}_CASES`, `_SEED`, `_STEPS`, `_JOBS` and `_SHRINK`.
pub struct Knobs {
    pub prefix: &'static str,
    pub label: &'static str,
    pub cases: u64,
    pub seed: u64,
    pub steps: u64,
    /// This run takes the cases whose index is `shard` modulo `shards`.
    pub shard: u64,
    pub shards: u64,
}

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

pub struct ModelHarness<R, E>(PhantomData<(R, E)>);

impl<R: Reference, E: Engine<R>> ModelHarness<R, E> {
    /// One case: every operation resolved against the model, run on both, then compared.
    pub async fn run_case(ops: &[R::Op]) -> Run {
        let mut engine = E::new().await;
        let run = Self::run_with(&mut engine, ops).await;
        // Dropping a test database blocks on DROP DATABASE, which waits for every backend,
        // including another case's connection still authenticating on this task.
        tokio::task::spawn_blocking(move || drop(engine))
            .await
            .expect("the test database dropped");
        run
    }

    async fn run_with(engine: &mut E, ops: &[R::Op]) -> Run {
        let mut model = R::default();
        let mut lines = vec![];
        let mut kinds = vec![];
        for (step, op) in ops.iter().enumerate() {
            let Some(action) = model.resolve(op) else {
                continue;
            };
            let mut details: Vec<Diff> = engine.before(&mut model, &action).await;
            lines.push(model.describe(&action));
            kinds.push(R::kind(&action));
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
                    format!("expected a refusal ({}), the engine accepted", R::reason(r)),
                )),
            }
            if let (Err(r), Err(e)) = (&expected, &actual) {
                lines.push(format!(
                    "        refused as expected ({}): {e}",
                    R::reason(r)
                ));
            }
            details.extend(engine.compare(&model).await);
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
            lines.push(model.state());
            if !details.is_empty() {
                let signature = format!("{}: {}", R::kind(&action), details[0].0);
                let mut details: Vec<String> = details.into_iter().map(|(_, d)| d).collect();
                details.extend(engine.dump().await);
                return Run {
                    lines,
                    kinds,
                    failure: Some(Failure {
                        step,
                        signature,
                        details,
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
    pub async fn shrink(ops: &[R::Op], failure: &Failure, budget: usize) -> (Vec<R::Op>, Run) {
        let mut best: Vec<R::Op> = ops[..=failure.step].to_vec();
        let mut best_run = Self::run_case(&best).await;
        let mut runs = 1;
        let mut i = best.len().saturating_sub(1);
        while i >= 1 && runs < budget {
            let mut candidate = best.clone();
            candidate.remove(i);
            let run = Self::run_case(&candidate).await;
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

    /// `{prefix}_CASES` cases from `{prefix}_SEED`, `{prefix}_STEPS` operations each,
    /// `{prefix}_JOBS` at a time; the first `{prefix}_SHRINK` distinct failures are shrunk and
    /// printed.
    pub async fn run_cases(knobs: Knobs) {
        let prefix = knobs.prefix;
        let var = |name: &str, default: u64| env(&format!("{prefix}_{name}"), default);
        let cases = var("CASES", knobs.cases);
        let base = var("SEED", knobs.seed);
        let steps = var("STEPS", knobs.steps) as usize;
        let jobs = var("JOBS", 4) as usize;
        let shrinks = var("SHRINK", 8) as usize;
        let started = Instant::now();
        let seeds: Vec<u64> = (0..cases)
            .filter(|i| i % knobs.shards == knobs.shard)
            .map(|i| base.wrapping_add(i))
            .collect();
        let results: Vec<(u64, Vec<R::Op>, Run)> = futures::stream::iter(seeds)
            .map(|seed| async move {
                let ops = R::generate(seed, steps);
                let run = Self::run_case(&ops).await;
                (seed, ops, run)
            })
            .buffer_unordered(jobs)
            .collect()
            .await;
        let mut failures: Vec<&(u64, Vec<R::Op>, Run)> = results
            .iter()
            .filter(|(_, _, r)| r.failure.is_some())
            .collect();
        failures.sort_by_key(|(seed, _, _)| *seed);
        eprintln!(
            "{} tests: {} cases from seed {base:#x}, {} failed, {:?}",
            knobs.label,
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
        let firsts: Vec<(String, u64, Vec<R::Op>, Failure)> = by_signature
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
                    let (_, run) = Self::shrink(&ops, &failure, 40).await;
                    let failure = run.failure.clone().unwrap_or(failure);
                    (run.lines, failure)
                } else {
                    let run = Self::run_case(&ops[..=failure.step]).await;
                    (run.lines, failure)
                };
                let mut report = vec![format!(
                    "\n=== {signature} (seed {seed:#x}, replay with {prefix}_SEED={seed:#x} {prefix}_CASES=1) ==="
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
            "{} of {} {} cases failed: {:?}",
            failures.len(),
            results.len(),
            knobs.label,
            by_signature.keys().collect::<Vec<_>>()
        );
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
