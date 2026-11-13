//! Model-based randomized tests of the billing engine: seeded operation sequences run against
//! the engine and against a small reference model, compared after every step with the invariants
//! and a second pass of every sweep and worker (`harness`). The credit model is sized by
//! `MODEL_CASES`, `MODEL_SEED`, `MODEL_STEPS`, `MODEL_JOBS` and `MODEL_SHRINK`, the invoice model
//! by the same names prefixed `INVOICE_`; every failure prints its seed.
mod engine;
mod generator;
mod harness;
mod invoice;
mod reference;
mod regressions;
mod runner;

/// The cases in four shards, so they spread over the test runner's slots. Pull requests run a
/// fixed handful of cases; the scheduled model-tests workflow explores fresh seeds.
#[rstest::rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[case(3)]
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
