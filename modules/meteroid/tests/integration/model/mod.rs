//! Model-based randomized tests of the billing engine: seeded operation sequences run against
//! the engine and against a small reference model, compared after every step with the invariants
//! and a second pass of every sweep and worker (`harness`). The credit model is sized by
//! `MODEL_CASES`, `MODEL_SEED`, `MODEL_STEPS`, `MODEL_JOBS` and `MODEL_SHRINK`, the others by the
//! same names prefixed `INVOICE_` and `LIFECYCLE_`; every failure prints its seed.
mod engine;
mod generator;
mod harness;
mod invoice;
mod lifecycle;
mod reference;
mod regressions;
mod runner;

/// The cases in shards, so they spread over the test runner's slots. Pull requests run a fixed
/// handful of cases; the scheduled model-tests workflow explores fresh seeds.
#[rstest::rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[case(3)]
fn credit_engine_matches_the_reference_model(#[case] shard: u64) {
    harness::on_large_stacks(move || runner::run_cases(shard, 4));
}

#[rstest::rstest]
#[case(0)]
#[case(1)]
fn invoice_engine_matches_the_reference_model(#[case] shard: u64) {
    harness::on_large_stacks(move || invoice::run_cases(shard, 2));
}

/// Each shard is a process of its own under nextest: its cases move the clock.
#[rstest::rstest]
#[case(0)]
#[case(1)]
fn lifecycle_bills_what_the_reference_owes(#[case] shard: u64) {
    harness::on_large_stacks(move || lifecycle::run_cases(shard, 2));
}
