use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CustomerId};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

#[derive(Debug, Clone)]
pub enum WindowSize {
    Minute,
    Hour,
    Day,
    Week,
}

/// Uniform result row of any metric/meter query.
#[derive(Debug, Clone)]
pub struct Usage {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub value: f64,
    pub customer_id: Option<CustomerId>,
    pub group_by: HashMap<String, Option<String>>,
}

/// Label a breakdown's overflow values collapse into under [`apply_top_n`].
pub const OTHER_LABEL: &str = "Other";

/// How a top-N cap ranks the series of a breakdown and what becomes of the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopNFold {
    /// Ranked by the sum of `|value|` over all windows; the rest sums into one "Other" row per
    /// window, keeping the total honest. Only for values that add up (sum, count).
    Additive,
    /// Ranked by the largest `|value|` of any window; the rest is dropped, since summing them
    /// (distinct counts, maxima, averages, latest values) would yield a meaningless figure.
    NonAdditive,
}

/// Cap a single-dimension breakdown to the `n` highest-magnitude values, bucketing
/// the rest under [`OTHER_LABEL`]. Each `dim` value is ranked by the sum of `|value|`
/// across all windows; the top `n` are kept, the rest are relabelled to "Other", and
/// rows that then collapse together are re-summed. `n == 0`, or `n` ≥ the distinct
/// value count, is a no-op. Rows missing the `dim` key pass through untouched.
///
/// The guard against a high-cardinality breakdown (e.g. group-by-customer) producing
/// thousands of series; "Other" keeps the total honest rather than silently dropping.
pub fn apply_top_n(rows: Vec<Usage>, dim: &str, n: usize) -> Vec<Usage> {
    apply_top_n_with(rows, dim, n, TopNFold::Additive)
        .into_iter()
        .map(|row| row.usage)
        .collect()
}

/// A row of a top-N result. `other` marks the folded overflow, which its labels alone cannot
/// tell apart from a real "Other" value.
#[derive(Debug, Clone)]
pub struct TopNRow {
    pub usage: Usage,
    pub other: bool,
}

/// [`apply_top_n`] with an explicit [`TopNFold`]. Ties rank by the `dim` value, ascending.
pub fn apply_top_n_with(rows: Vec<Usage>, dim: &str, n: usize, fold: TopNFold) -> Vec<TopNRow> {
    fold_top_n(
        rows,
        n,
        fold,
        |r| r.group_by.get(dim).cloned().flatten(),
        |r| {
            if let Some(slot) = r.group_by.get_mut(dim) {
                *slot = Some(OTHER_LABEL.to_string());
            }
        },
    )
}

/// Cap a per-customer breakdown to the `n` largest customers. Under [`TopNFold::Additive`]
/// the rest merges into one row per window (and remaining dims) with no `customer_id`. Ties
/// rank by customer id, ascending. Rows without a customer pass through untouched.
pub fn apply_top_n_by_customer(rows: Vec<Usage>, n: usize, fold: TopNFold) -> Vec<TopNRow> {
    fold_top_n(
        rows,
        n,
        fold,
        |r| r.customer_id.map(|id| id.as_uuid()),
        |_| {},
    )
}

fn fold_top_n<K, Key, Relabel>(
    rows: Vec<Usage>,
    n: usize,
    fold: TopNFold,
    key: Key,
    relabel: Relabel,
) -> Vec<TopNRow>
where
    K: Eq + Hash + Ord,
    Key: Fn(&Usage) -> Option<K>,
    Relabel: Fn(&mut Usage),
{
    let untouched = |rows: Vec<Usage>| {
        rows.into_iter()
            .map(|usage| TopNRow {
                usage,
                other: false,
            })
            .collect()
    };
    if n == 0 {
        return untouched(rows);
    }
    let mut scores: HashMap<K, f64> = HashMap::new();
    for r in &rows {
        if let Some(k) = key(r) {
            let score = scores.entry(k).or_default();
            match fold {
                TopNFold::Additive => *score += r.value.abs(),
                TopNFold::NonAdditive => *score = score.max(r.value.abs()),
            }
        }
    }
    if scores.len() <= n {
        return untouched(rows);
    }
    // Rank by magnitude desc, tie-break by key asc for determinism.
    let mut ranked: Vec<(K, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    let keep: HashSet<K> = ranked.into_iter().take(n).map(|(k, _)| k).collect();

    // Kept rows (a top-N key, or no key) pass through untouched in input order;
    // only the overflow is relabelled to "Other" and merged on window + remaining dims.
    let is_overflow = |r: &Usage| key(r).is_some_and(|k| !keep.contains(&k));
    // Merge key: (window_start, window_end, sorted remaining dims).
    type MergeKey = (i64, i64, Vec<(String, Option<String>)>);
    let mut kept: Vec<TopNRow> = Vec::new();
    let mut merged: HashMap<MergeKey, Usage> = HashMap::new();
    for mut r in rows {
        if !is_overflow(&r) {
            kept.push(TopNRow {
                usage: r,
                other: false,
            });
            continue;
        }
        if fold == TopNFold::NonAdditive {
            continue;
        }
        relabel(&mut r);
        // The Other bucket aggregates many customers, so it has no single owner — drop
        // any per-customer id a breakdown row carried, so callers can't mis-link it.
        r.customer_id = None;
        let mut dims: Vec<(String, Option<String>)> = r
            .group_by
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        dims.sort();
        let key = (r.window_start.timestamp(), r.window_end.timestamp(), dims);
        merged
            .entry(key)
            .and_modify(|acc| acc.value += r.value)
            .or_insert(r);
    }

    // Append the merged "Other" rows in a deterministic (window, remaining dims) order.
    let mut other: Vec<(MergeKey, Usage)> = merged.into_iter().collect();
    other.sort_by(|a, b| a.0.cmp(&b.0));
    kept.extend(
        other
            .into_iter()
            .map(|(_, usage)| TopNRow { usage, other: true }),
    );
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::collections::HashSet;

    fn row(win: i64, plan: &str, value: f64) -> Usage {
        Usage {
            window_start: Utc.timestamp_opt(win, 0).unwrap(),
            window_end: Utc.timestamp_opt(win + 86_400, 0).unwrap(),
            value,
            customer_id: None,
            group_by: HashMap::from([("plan".to_string(), Some(plan.to_string()))]),
        }
    }

    #[test]
    fn top_n_keeps_top_and_merges_overflow_into_other() {
        // Totals: big=300, mid=120, small=30, tiny=6 → top 2 = big, mid; small+tiny → Other.
        let rows = vec![
            row(0, "big", 200.0),
            row(86_400, "big", 100.0),
            row(0, "mid", 80.0),
            row(86_400, "mid", 40.0),
            row(0, "small", 20.0),
            row(86_400, "small", 10.0),
            row(0, "tiny", 4.0),
            row(86_400, "tiny", 2.0),
        ];
        let out = apply_top_n(rows, "plan", 2);

        let labels: HashSet<String> = out
            .iter()
            .filter_map(|r| r.group_by.get("plan").and_then(|o| o.clone()))
            .collect();
        assert_eq!(
            labels,
            HashSet::from(["big".into(), "mid".into(), OTHER_LABEL.into()])
        );

        // The two overflow labels merge into one "Other" row per window: w0 = 20 + 4.
        let other_w0: f64 = out
            .iter()
            .filter(|r| {
                r.window_start.timestamp() == 0
                    && r.group_by.get("plan").and_then(|o| o.clone()).as_deref()
                        == Some(OTHER_LABEL)
            })
            .map(|r| r.value)
            .sum();
        assert_eq!(other_w0, 24.0);
        let other_rows = out
            .iter()
            .filter(|r| {
                r.window_start.timestamp() == 0
                    && r.group_by.get("plan").and_then(|o| o.clone()).as_deref()
                        == Some(OTHER_LABEL)
            })
            .count();
        assert_eq!(
            other_rows, 1,
            "overflow collapses to a single Other row per window"
        );
    }

    #[test]
    fn top_n_is_noop_within_limit() {
        let rows = vec![row(0, "a", 1.0), row(0, "b", 2.0)];
        assert_eq!(apply_top_n(rows, "plan", 5).len(), 2);
    }

    #[test]
    fn top_n_clears_customer_id_on_the_other_bucket() {
        // A per-customer breakdown row owns a customer_id. The kept top rows keep theirs;
        // the collapsed Other bucket aggregates many, so it must carry none.
        let cust = |name: &str, n: u128, value: f64| Usage {
            window_start: Utc.timestamp_opt(0, 0).unwrap(),
            window_end: Utc.timestamp_opt(86_400, 0).unwrap(),
            value,
            customer_id: Some(CustomerId::from(uuid::Uuid::from_u128(n))),
            group_by: HashMap::from([("customer".to_string(), Some(name.to_string()))]),
        };
        let rows = vec![
            cust("big", 1, 300.0),
            cust("mid", 2, 120.0),
            cust("small", 3, 30.0),
            cust("tiny", 4, 6.0),
        ];
        let out = apply_top_n(rows, "customer", 2);
        for r in &out {
            let is_other = r
                .group_by
                .get("customer")
                .and_then(|o| o.clone())
                .as_deref()
                == Some(OTHER_LABEL);
            if is_other {
                assert!(
                    r.customer_id.is_none(),
                    "Other bucket carries no customer id"
                );
            } else {
                assert!(r.customer_id.is_some(), "a kept customer keeps its id");
            }
        }
    }

    fn customer_row(win: i64, customer: u128, value: f64) -> Usage {
        Usage {
            window_start: Utc.timestamp_opt(win, 0).unwrap(),
            window_end: Utc.timestamp_opt(win + 86_400, 0).unwrap(),
            value,
            customer_id: Some(CustomerId::from(uuid::Uuid::from_u128(customer))),
            group_by: HashMap::new(),
        }
    }

    fn customers(rows: &[TopNRow]) -> Vec<Option<u128>> {
        rows.iter()
            .map(|r| {
                assert_eq!(r.other, r.usage.customer_id.is_none());
                r.usage.customer_id.map(|id| id.as_uuid().as_u128())
            })
            .collect()
    }

    #[test]
    fn top_n_by_customer_additive_folds_the_rest_into_one_row_per_window() {
        // Totals: 1=300, 2=120, 3=30, 4=6 → keep 1 and 2; 3+4 merge per window.
        let rows = vec![
            customer_row(0, 1, 200.0),
            customer_row(86_400, 1, 100.0),
            customer_row(0, 2, 80.0),
            customer_row(86_400, 2, 40.0),
            customer_row(0, 3, 20.0),
            customer_row(86_400, 3, 10.0),
            customer_row(0, 4, 4.0),
            customer_row(86_400, 4, 2.0),
        ];
        let out = apply_top_n_by_customer(rows, 2, TopNFold::Additive);

        assert_eq!(
            customers(&out),
            vec![Some(1), Some(1), Some(2), Some(2), None, None]
        );
        let other: Vec<(i64, f64)> = out
            .iter()
            .filter(|r| r.other)
            .map(|r| (r.usage.window_start.timestamp(), r.usage.value))
            .collect();
        assert_eq!(other, vec![(0, 24.0), (86_400, 12.0)]);
        let total: f64 = out.iter().map(|r| r.usage.value).sum();
        assert_eq!(total, 456.0, "the fold keeps the total");
    }

    #[test]
    fn top_n_by_customer_non_additive_drops_the_rest_and_ranks_by_peak() {
        // Sums would rank 3 first (3 × 50 = 150 vs 1 → 90, 2 → 80); peaks rank 1 (90), 2 (80).
        let rows = vec![
            customer_row(0, 1, 90.0),
            customer_row(0, 2, 80.0),
            customer_row(0, 3, 50.0),
            customer_row(86_400, 3, 50.0),
            customer_row(172_800, 3, 50.0),
            customer_row(0, 4, 10.0),
        ];
        let out = apply_top_n_by_customer(rows, 2, TopNFold::NonAdditive);
        assert_eq!(customers(&out), vec![Some(1), Some(2)]);
    }

    #[test]
    fn top_n_by_customer_breaks_ties_by_customer_id() {
        let rows = vec![
            customer_row(0, 3, 10.0),
            customer_row(0, 2, 10.0),
            customer_row(0, 1, 10.0),
        ];
        let out = apply_top_n_by_customer(rows.clone(), 2, TopNFold::NonAdditive);
        assert_eq!(customers(&out), vec![Some(2), Some(1)]);

        let out = apply_top_n_by_customer(rows, 2, TopNFold::Additive);
        assert_eq!(customers(&out), vec![Some(2), Some(1), None]);
        assert_eq!(out[2].usage.value, 10.0);
    }

    #[test]
    fn top_n_by_customer_is_noop_within_limit() {
        let rows = vec![customer_row(0, 1, 1.0), customer_row(0, 2, 2.0)];
        let out = apply_top_n_by_customer(rows, 2, TopNFold::Additive);
        assert_eq!(customers(&out), vec![Some(1), Some(2)]);
    }

    #[test]
    fn top_n_by_property_non_additive_drops_the_rest_without_other() {
        let rows = vec![
            row(0, "a", 5.0),
            row(0, "b", 7.0),
            row(86_400, "b", 1.0),
            row(0, "c", 7.0),
        ];
        let out = apply_top_n_with(rows, "plan", 2, TopNFold::NonAdditive);
        let labels: Vec<String> = out
            .iter()
            .filter_map(|r| r.usage.group_by.get("plan").cloned().flatten())
            .collect();
        // b and c tie on their peak (7); a is dropped and no Other row appears.
        assert_eq!(labels, vec!["b", "b", "c"]);
        assert!(out.iter().all(|r| !r.other));
    }

    #[test]
    fn top_n_marks_the_fold_apart_from_a_real_other_value() {
        let rows = vec![
            row(0, OTHER_LABEL, 50.0),
            row(0, "big", 100.0),
            row(0, "small", 1.0),
            row(0, "tiny", 2.0),
        ];
        let out = apply_top_n_with(rows, "plan", 2, TopNFold::Additive);
        let rows: Vec<(Option<String>, f64, bool)> = out
            .iter()
            .map(|r| {
                (
                    r.usage.group_by.get("plan").cloned().flatten(),
                    r.usage.value,
                    r.other,
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                (Some(OTHER_LABEL.to_string()), 50.0, false),
                (Some("big".to_string()), 100.0, false),
                (Some(OTHER_LABEL.to_string()), 3.0, true),
            ]
        );
    }

    #[test]
    fn top_n_by_property_ties_break_by_value() {
        let rows = vec![row(0, "z", 3.0), row(0, "y", 3.0), row(0, "x", 3.0)];
        let out = apply_top_n(rows, "plan", 1);
        let labels: Vec<String> = out
            .iter()
            .filter_map(|r| r.group_by.get("plan").cloned().flatten())
            .collect();
        assert_eq!(labels, vec!["x".to_string(), OTHER_LABEL.to_string()]);
    }
}
