//! What a line bills, as a key that stays the same however often the line is computed: the
//! subscription, the fee's lineage, the period and the kind of charge (R-075).

use chrono::NaiveDate;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

/// The fee a charge comes from, by the root of its lineage, so an override bills under the key
/// of the fee it replaced.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Fee {
    Component(Uuid),
    AddOn(Uuid),
    /// The subscription's minimum commitment, by the product its shortfall is split to.
    Minimum {
        product: Option<Uuid>,
    },
    /// A line attached to no fee of the subscription.
    Unattached,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChargeKind {
    Fee,
    Usage {
        metric: Uuid,
        dimensions: BTreeMap<String, String>,
    },
    Shortfall,
    Credit,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChargeKey {
    pub subscription: Option<Uuid>,
    pub fee: Fee,
    /// The period the charge bills, `[from, until)`.
    pub from: NaiveDate,
    pub until: NaiveDate,
    pub kind: ChargeKind,
}

const ID_LENGTH: u32 = 13;
const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

impl ChargeKey {
    /// The key as text, versioned: a change to it changes every id, so it only ever grows.
    fn canonical(&self) -> String {
        let subscription = self
            .subscription
            .map(|id| id.to_string())
            .unwrap_or_default();
        let fee = match &self.fee {
            Fee::Component(id) => format!("component:{id}"),
            Fee::AddOn(id) => format!("add_on:{id}"),
            Fee::Minimum { product: None } => "minimum".to_owned(),
            Fee::Minimum { product: Some(id) } => format!("minimum:{id}"),
            Fee::Unattached => "unattached".to_owned(),
        };
        let kind = match &self.kind {
            ChargeKind::Fee => "fee".to_owned(),
            ChargeKind::Usage { metric, dimensions } => {
                let dimensions: Vec<String> =
                    dimensions.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("usage:{metric}:{}", dimensions.join("&"))
            }
            ChargeKind::Shortfall => "shortfall".to_owned(),
            ChargeKind::Credit => "credit".to_owned(),
        };
        format!(
            "v1|{subscription}|{fee}|{}..{}|{kind}",
            self.from, self.until
        )
    }

    /// The line id of the charge: 13 base62 characters, like the ids lines had before.
    pub fn line_id(&self) -> String {
        id_of(&self.canonical())
    }
}

fn id_of(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut n = u128::from_be_bytes(digest[..16].try_into().expect("16 bytes"));
    let mut id = vec![b'0'; ID_LENGTH as usize];
    for slot in id.iter_mut().rev() {
        *slot = ALPHABET[(n % 62) as usize];
        n /= 62;
    }
    String::from_utf8(id).expect("ASCII")
}

/// The ids of `keys` on one document whose `taken` ids are already in use: the key's id, and for
/// a key seen again (or an id taken) the id of the key and its occurrence, in order.
pub fn line_ids<'a>(
    keys: impl IntoIterator<Item = &'a ChargeKey>,
    taken: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut used: HashSet<String> = taken.into_iter().collect();
    keys.into_iter()
        .map(|key| {
            let canonical = key.canonical();
            let mut id = id_of(&canonical);
            let mut occurrence = 1;
            while !used.insert(id.clone()) {
                occurrence += 1;
                id = id_of(&format!("{canonical}#{occurrence}"));
            }
            id
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(from: (i32, u32, u32)) -> ChargeKey {
        ChargeKey {
            subscription: Some(Uuid::from_u128(1)),
            fee: Fee::Component(Uuid::from_u128(2)),
            from: NaiveDate::from_ymd_opt(from.0, from.1, from.2).unwrap(),
            until: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            kind: ChargeKind::Fee,
        }
    }

    #[test]
    fn the_same_charge_gets_the_same_id_and_another_charge_another() {
        let id = key((2024, 1, 1)).line_id();
        assert_eq!(id, key((2024, 1, 1)).line_id());
        assert_eq!(id.len(), 13);
        assert!(id.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_ne!(id, key((2024, 1, 2)).line_id());
    }

    #[test]
    fn the_id_is_pinned() {
        assert_eq!(
            key((2024, 1, 1)).line_id(),
            id_of(
                "v1|00000000-0000-0000-0000-000000000001|component:00000000-0000-0000-0000-000000000002|2024-01-01..2024-02-01|fee"
            )
        );
    }

    #[test]
    fn dimensions_are_keyed_in_order() {
        let usage = |pairs: &[(&str, &str)]| ChargeKey {
            kind: ChargeKind::Usage {
                metric: Uuid::from_u128(3),
                dimensions: pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            },
            ..key((2024, 1, 1))
        };
        assert_eq!(
            usage(&[("a", "1"), ("b", "2")]).line_id(),
            usage(&[("b", "2"), ("a", "1")]).line_id()
        );
        assert_ne!(
            usage(&[("a", "1")]).line_id(),
            usage(&[("a", "2")]).line_id()
        );
    }

    #[test]
    fn a_repeated_key_or_a_taken_id_gets_its_occurrence() {
        let k = key((2024, 1, 1));
        let ids = line_ids([&k, &k], []);
        assert_eq!(ids[0], k.line_id());
        assert_ne!(ids[0], ids[1]);
        assert_eq!(ids, line_ids([&k, &k], []));
        let after_taken = line_ids([&k], [k.line_id()]);
        assert_eq!(after_taken[0], ids[1]);
    }
}
