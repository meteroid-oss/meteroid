//! Which entry of a rate card rates a usage row: a key per dimension, dated entries per key.
//! Rules: R-131.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::NaiveDate;

/// What one dimension of an entry's key matches.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pattern {
    /// Any value, the dimension absent included: `*`.
    Any,
    /// Values starting with it: `anthropic/*`.
    Prefix(String),
    Exact(String),
}

impl Pattern {
    pub fn parse(pattern: &str) -> Pattern {
        match pattern.strip_suffix('*') {
            Some("") => Pattern::Any,
            Some(prefix) => Pattern::Prefix(prefix.to_owned()),
            None => Pattern::Exact(pattern.to_owned()),
        }
    }

    pub fn as_text(&self) -> String {
        match self {
            Pattern::Any => "*".to_owned(),
            Pattern::Prefix(prefix) => format!("{prefix}*"),
            Pattern::Exact(value) => value.clone(),
        }
    }

    fn matches(&self, value: Option<&str>) -> bool {
        match (self, value) {
            (Pattern::Any, _) => true,
            (Pattern::Prefix(prefix), Some(value)) => value.starts_with(prefix.as_str()),
            (Pattern::Exact(exact), Some(value)) => exact == value,
            (_, None) => false,
        }
    }

    /// An exact value beats a prefix, a longer prefix a shorter one, any prefix `*`.
    fn specificity(&self) -> (u8, usize) {
        match self {
            Pattern::Any => (0, 0),
            Pattern::Prefix(prefix) => (1, prefix.len()),
            Pattern::Exact(_) => (2, 0),
        }
    }
}

/// A rate for the usage its key matches, from `effective_from` until a later entry of the same
/// key; `key` holds one pattern per dimension of the card, in the card's order. A `rate` of None
/// removes the key from its day: usage it matched falls to the next key matching it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry<T> {
    pub key: Vec<Pattern>,
    pub effective_from: NaiveDate,
    pub rate: Option<T>,
}

/// The rate of usage carrying `dimensions` on `on`: of the keys matching it and not removed, the
/// one most specific on the card's first dimension, then the next; of that key's entries, the
/// latest in force. None when no key matches: the usage is unrated.
pub fn resolve<'a, T>(
    card_dimensions: &[String],
    entries: &'a [Entry<T>],
    dimensions: &BTreeMap<String, String>,
    on: NaiveDate,
) -> Option<&'a T> {
    let values: Vec<Option<&str>> = card_dimensions
        .iter()
        .map(|dimension| dimensions.get(dimension).map(String::as_str))
        .collect();
    let matching: Vec<&Entry<T>> = entries
        .iter()
        .filter(|entry| entry.effective_from <= on && entry.key.len() == values.len())
        .filter(|entry| {
            entry
                .key
                .iter()
                .zip(&values)
                .all(|(pattern, value)| pattern.matches(*value))
        })
        .collect();
    let mut removed: Vec<&[Pattern]> = Vec::new();
    loop {
        // Keys matching one row never tie on specificity: the max is one key's latest entry.
        let latest: &'a Entry<T> = matching
            .iter()
            .copied()
            .filter(|entry| !removed.contains(&entry.key.as_slice()))
            .max_by(|a, b| {
                compare_specificity(&a.key, &b.key).then(a.effective_from.cmp(&b.effective_from))
            })?;
        match &latest.rate {
            Some(rate) => return Some(rate),
            None => removed.push(&latest.key),
        }
    }
}

/// Whether some usage matches both keys.
pub fn overlaps(a: &[Pattern], b: &[Pattern]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|pair| match pair {
            (Pattern::Any, _) | (_, Pattern::Any) => true,
            (Pattern::Exact(x), Pattern::Exact(y)) => x == y,
            (Pattern::Exact(x), Pattern::Prefix(p)) | (Pattern::Prefix(p), Pattern::Exact(x)) => {
                x.starts_with(p.as_str())
            }
            (Pattern::Prefix(p), Pattern::Prefix(q)) => {
                p.starts_with(q.as_str()) || q.starts_with(p.as_str())
            }
        })
}

/// Whether `a` rates the usage both keys match, as `resolve` decides.
pub fn beats(a: &[Pattern], b: &[Pattern]) -> bool {
    compare_specificity(a, b) == Ordering::Greater
}

fn compare_specificity(a: &[Pattern], b: &[Pattern]) -> Ordering {
    a.iter()
        .map(Pattern::specificity)
        .cmp(b.iter().map(Pattern::specificity))
}

/// The days in `[from, until)` on which an entry matching some key of `keys` takes over: where
/// the usage of those keys is to be cut, never where only another key's rate changed.
pub fn cuts<T>(
    entries: &[Entry<T>],
    keys: &[Vec<Pattern>],
    from: NaiveDate,
    until: NaiveDate,
) -> Vec<NaiveDate> {
    let mut days: Vec<NaiveDate> = entries
        .iter()
        .filter(|entry| keys.contains(&entry.key))
        .map(|entry| entry.effective_from)
        .filter(|day| from < *day && *day < until)
        .collect();
    days.sort();
    days.dedup();
    days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn entry(key: &[&str], from: u32, rate: u32) -> Entry<u32> {
        Entry {
            key: key.iter().map(|p| Pattern::parse(p)).collect(),
            effective_from: day(from),
            rate: Some(rate),
        }
    }

    fn removal(key: &[&str], from: u32) -> Entry<u32> {
        Entry {
            rate: None,
            ..entry(key, from, 0)
        }
    }

    fn usage(model: &str, kind: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("model".to_owned(), model.to_owned()),
            ("token_type".to_owned(), kind.to_owned()),
        ])
    }

    fn dimensions() -> Vec<String> {
        vec!["model".to_owned(), "token_type".to_owned()]
    }

    #[test]
    fn patterns_parse_to_exact_prefix_or_any() {
        assert_eq!(Pattern::parse("*"), Pattern::Any);
        assert_eq!(
            Pattern::parse("anthropic/*"),
            Pattern::Prefix("anthropic/".into())
        );
        assert_eq!(Pattern::parse("gpt-5"), Pattern::Exact("gpt-5".into()));
        assert_eq!(Pattern::parse("anthropic/*").as_text(), "anthropic/*");
    }

    /// `(X, *)` and `(*, cached)` both match `(X, cached)`: the card's first dimension decides.
    #[test]
    fn the_first_dimension_decides_between_equally_specific_keys() {
        let entries = vec![entry(&["*", "cached"], 1, 1), entry(&["x", "*"], 1, 2)];
        let found = resolve(&dimensions(), &entries, &usage("x", "cached"), day(10));
        assert_eq!(found.copied(), Some(2));
    }

    #[test]
    fn an_exact_value_beats_a_prefix_and_a_longer_prefix_a_shorter_one() {
        let entries = vec![
            entry(&["*", "input"], 1, 1),
            entry(&["anthropic/*", "input"], 1, 2),
            entry(&["anthropic/claude-*", "input"], 1, 3),
            entry(&["anthropic/claude-x", "input"], 1, 4),
        ];
        let rate =
            |model| resolve(&dimensions(), &entries, &usage(model, "input"), day(10)).copied();
        assert_eq!(rate("anthropic/claude-x"), Some(4));
        assert_eq!(rate("anthropic/claude-y"), Some(3));
        assert_eq!(rate("anthropic/other"), Some(2));
        assert_eq!(rate("openai/gpt"), Some(1));
    }

    #[test]
    fn a_key_is_rated_by_its_latest_entry_in_force() {
        let entries = vec![
            entry(&["x", "input"], 1, 10),
            entry(&["x", "input"], 15, 12),
        ];
        let rate = |on| resolve(&dimensions(), &entries, &usage("x", "input"), day(on)).copied();
        assert_eq!(rate(14), Some(10));
        assert_eq!(rate(15), Some(12));
    }

    /// Removed from the 15th, x falls to `*`; before, it keeps its rate.
    #[test]
    fn a_removed_key_leaves_its_usage_to_the_next_key_matching_it() {
        let entries = vec![
            entry(&["*", "*"], 1, 1),
            entry(&["x", "input"], 1, 10),
            removal(&["x", "input"], 15),
        ];
        let rate = |on| resolve(&dimensions(), &entries, &usage("x", "input"), day(on)).copied();
        assert_eq!(rate(14), Some(10));
        assert_eq!(rate(15), Some(1));
        let alone = vec![entry(&["x", "input"], 1, 10), removal(&["x", "input"], 15)];
        assert!(resolve(&dimensions(), &alone, &usage("x", "input"), day(15)).is_none());
    }

    #[test]
    fn usage_no_key_matches_is_unrated() {
        let entries = vec![entry(&["x", "input"], 1, 10)];
        assert!(resolve(&dimensions(), &entries, &usage("y", "input"), day(10)).is_none());
        assert!(resolve(&dimensions(), &entries, &usage("x", "input"), day(1)).is_some());
        assert!(
            resolve(&dimensions(), &entries, &BTreeMap::new(), day(10)).is_none(),
            "a missing dimension matches only *"
        );
    }

    #[test]
    fn keys_overlap_when_some_usage_matches_both() {
        let key = |patterns: &[&str]| -> Vec<Pattern> {
            patterns.iter().map(|p| Pattern::parse(p)).collect()
        };
        assert!(overlaps(&key(&["a/*", "input"]), &key(&["a/x", "*"])));
        assert!(overlaps(&key(&["a/*"]), &key(&["a/b*"])));
        assert!(!overlaps(&key(&["a/*"]), &key(&["b/x"])));
        assert!(!overlaps(&key(&["x", "input"]), &key(&["x", "output"])));
        assert!(beats(&key(&["a/x", "*"]), &key(&["*", "input"])));
    }

    #[test]
    fn only_the_keys_used_cut_the_window() {
        let entries = vec![
            entry(&["x", "input"], 1, 10),
            entry(&["x", "input"], 15, 12),
            entry(&["y", "input"], 20, 5),
        ];
        let used = vec![entries[0].key.clone()];
        assert_eq!(cuts(&entries, &used, day(1), day(30)), vec![day(15)]);
    }
}
