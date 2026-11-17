//! Model-based randomized tests of the billing engine: seeded operation sequences run against
//! the engine and against a small reference model, compared after every step with the invariants
//! and a second pass of every sweep and worker (`harness`). The credit model is sized by
//! `MODEL_CASES`, `MODEL_SEED`, `MODEL_STEPS`, `MODEL_JOBS` and `MODEL_SHRINK`, the others by the
//! same names prefixed `INVOICE_` and `LIFECYCLE_`, the lifecycle's shards by
//! `LIFECYCLE_MODEL_SHARDS`; every failure prints its seed, `MODEL_PROFILE=1` each phase's time.
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

/// Each shard is a process of its own under nextest, as its cases move the clock: one core each.
/// `LIFECYCLE_MODEL_SHARDS` (at most 16) splits the cases; the shards past it run none.
#[rstest::rstest]
fn lifecycle_bills_what_the_reference_owes(
    #[values(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15)] shard: u64,
) {
    let shards = harness::shards("LIFECYCLE_MODEL", 16);
    if shard < shards {
        harness::on_large_stacks(move || lifecycle::run_cases(shard, shards));
    }
}
