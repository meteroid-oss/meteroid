//! One snapshot per corpus case, in the review form of [`crate::testing::snapshot`]; the cases
//! the compiler refuses share one snapshot of their errors.

use crate::sql::compiler::{compile_cohort_members_tree, compile_tree};
use crate::testing::{
    member_cases, metric_cases, query, registry, snapshot, snapshot_name, tenant,
};
use std::fmt::Write;

#[cfg(test)]
const EVENTS: &str = "raw_events_v2";

#[test]
fn compiled_metric_queries() {
    let registry = registry();
    let mut refused = String::new();
    for (name, q) in metric_cases() {
        match compile_tree(&registry, &q, EVENTS, "meteroid") {
            Ok((tree, settings)) => insta::assert_snapshot!(
                snapshot_name(&name),
                snapshot(&tree, &settings, *tenant()),
                &name
            ),
            Err(e) => writeln!(refused, "{name}: {e}").unwrap(),
        }
    }
    insta::assert_snapshot!("refused_metric_queries", refused);
}

#[test]
fn compiled_cohort_members() {
    let registry = registry();
    let mut refused = String::new();
    for keep_last in [false, true] {
        for (name, def) in member_cases() {
            let name = format!("members {name} keep_last={keep_last}");
            let mut q = query("raw_sum");
            q.keep_last_events = keep_last;
            match compile_cohort_members_tree(&registry, &def, &q, EVENTS, "meteroid") {
                Ok(tree) => insta::assert_snapshot!(
                    snapshot_name(&name),
                    snapshot(&tree, &[], *tenant()),
                    &name
                ),
                Err(e) => writeln!(refused, "{name}: {e}").unwrap(),
            }
        }
    }
    insta::assert_snapshot!("refused_cohort_members", refused);
}
