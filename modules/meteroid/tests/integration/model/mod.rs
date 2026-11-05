//! Model-based randomized tests of the credit billing engine: seeded operation sequences run
//! against the engine and against a small reference model of the billing rules, compared after every step with the credit invariants and a second pass of every
//! sweep and worker. `MODEL_CASES`, `MODEL_SEED`, `MODEL_STEPS`, `MODEL_JOBS` and `MODEL_SHRINK`
//! size a run; every failure prints its seed.
mod engine;
mod generator;
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
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn credit_engine_matches_the_reference_model(#[case] shard: u64) {
    runner::run_cases(shard, 4).await;
}
