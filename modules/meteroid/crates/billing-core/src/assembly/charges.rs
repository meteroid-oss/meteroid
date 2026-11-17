//! The charge a line bills, read from what the line carries (R-075).

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::NaiveDate;
use uuid::Uuid;

use crate::charge::{ChargeKey, ChargeKind, Denomination, Fee, line_ids};

/// The lineage root of fee rows, by row id; a row it does not know is its own root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lineages(pub HashMap<Uuid, Uuid>);

impl Lineages {
    pub fn root(&self, row: Uuid) -> Uuid {
        self.0.get(&row).copied().unwrap_or(row)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Regular,
    Credit,
    MinimumShortfall,
}

/// What a line carries that names its charge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChargedLine {
    pub kind: LineKind,
    pub subscription: Option<Uuid>,
    pub component: Option<Uuid>,
    pub add_on: Option<Uuid>,
    pub product: Option<Uuid>,
    pub metric: Option<Uuid>,
    pub dimensions: BTreeMap<String, String>,
    pub from: NaiveDate,
    pub until: NaiveDate,
    /// The unit a usage line's price is in, when credits.
    pub credit_unit: Option<Uuid>,
}

pub fn charge_key(line: &ChargedLine, lineages: &Lineages) -> ChargeKey {
    let fee = match (&line.kind, line.component, line.add_on) {
        (LineKind::MinimumShortfall, _, _) => Fee::Minimum {
            product: line.product,
        },
        (_, Some(component), _) => Fee::Component(lineages.root(component)),
        (_, None, Some(add_on)) => Fee::AddOn(lineages.root(add_on)),
        (_, None, None) => Fee::Unattached,
    };
    let kind = match (&line.kind, line.metric) {
        (LineKind::MinimumShortfall, _) => ChargeKind::Shortfall,
        (LineKind::Credit, _) => ChargeKind::Credit,
        (LineKind::Regular, Some(metric)) => ChargeKind::Usage {
            metric,
            dimensions: line.dimensions.clone(),
        },
        (LineKind::Regular, None) => ChargeKind::Fee,
    };
    ChargeKey {
        subscription: line.subscription,
        fee,
        from: line.from,
        until: line.until,
        kind,
        denomination: line
            .credit_unit
            .map_or(Denomination::Money, Denomination::CreditUnit),
    }
}

/// The id of a line's `n`th sub-line (from 0).
pub fn sub_line_id(line_id: &str, n: usize) -> String {
    format!("{}.{}", line_id, n + 1)
}

/// The lines of a document, by position, whose id an earlier line already has, with the id each
/// takes instead: its key's next occurrence. `key` reads the charge of a line by its position.
pub fn disambiguate(ids: &[&str], key: impl Fn(usize) -> ChargeKey) -> Vec<(usize, String)> {
    let mut seen = HashSet::new();
    let repeated: Vec<usize> = (0..ids.len())
        .filter(|&i| !seen.insert(ids[i].to_owned()))
        .collect();
    if repeated.is_empty() {
        return Vec::new();
    }
    let keys: Vec<ChargeKey> = repeated.iter().map(|&i| key(i)).collect();
    repeated.into_iter().zip(line_ids(&keys, seen)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seats(component: Uuid) -> ChargedLine {
        let day = |d| NaiveDate::from_ymd_opt(2026, 1, d).unwrap();
        ChargedLine {
            kind: LineKind::Regular,
            subscription: Some(Uuid::from_u128(9)),
            component: Some(component),
            add_on: None,
            product: None,
            metric: None,
            dimensions: BTreeMap::new(),
            from: day(1),
            until: day(31),
            credit_unit: None,
        }
    }

    #[test]
    fn a_row_bills_under_its_lineage_root() {
        let (row, root) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let lineages = Lineages(HashMap::from([(row, root)]));
        assert_eq!(charge_key(&seats(row), &lineages).fee, Fee::Component(root));
        assert_eq!(
            charge_key(&seats(root), &Lineages::default()),
            charge_key(&seats(row), &lineages)
        );
    }

    #[test]
    fn a_repeated_id_takes_its_next_occurrence() {
        let line = seats(Uuid::from_u128(1));
        let id = charge_key(&line, &Lineages::default()).line_id();
        let renamed = disambiguate(&[&id, "other", &id], |_| {
            charge_key(&line, &Lineages::default())
        });
        assert_eq!(renamed.len(), 1);
        assert_eq!(renamed[0].0, 2);
        assert_ne!(renamed[0].1, id);
        assert!(disambiguate(&[&id, "other"], |_| unreachable!()).is_empty());
    }
}
