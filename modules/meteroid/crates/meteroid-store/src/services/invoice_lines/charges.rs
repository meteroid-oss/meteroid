//! Line ids from the charge a line bills (R-075).

use std::collections::{BTreeMap, HashMap, HashSet};

use billing_core::{ChargeKey, ChargeKind, Fee};
use common_domain::ids::BaseId;
use uuid::Uuid;

use crate::domain::{LineItem, LineItemKind, SubscriptionDetails};

/// The lineage root of each component and add-on of a subscription, by row id.
pub(crate) struct Lineages(HashMap<Uuid, Uuid>);

impl Lineages {
    pub(crate) fn of(details: &SubscriptionDetails) -> Self {
        let components = details
            .price_components
            .iter()
            .map(|c| (c.id.as_uuid(), c.lineage().as_uuid()));
        let add_ons = details
            .add_ons
            .iter()
            .map(|a| (a.id.as_uuid(), a.lineage().as_uuid()));
        Self(components.chain(add_ons).collect())
    }

    pub(crate) fn none() -> Self {
        Self(HashMap::new())
    }

    fn root(&self, row: Uuid) -> Uuid {
        self.0.get(&row).copied().unwrap_or(row)
    }
}

pub(crate) fn charge_key(line: &LineItem, lineages: &Lineages) -> ChargeKey {
    let fee = match (&line.kind, line.sub_component_id, line.sub_add_on_id) {
        (LineItemKind::MinimumShortfall, _, _) => Fee::Minimum {
            product: line.product_id.map(|id| id.as_uuid()),
        },
        (_, Some(component), _) => Fee::Component(lineages.root(component.as_uuid())),
        (_, None, Some(add_on)) => Fee::AddOn(lineages.root(add_on.as_uuid())),
        (_, None, None) => Fee::Unattached,
    };
    let kind = match (&line.kind, line.metric_id) {
        (LineItemKind::MinimumShortfall, _) => ChargeKind::Shortfall,
        (LineItemKind::Credit, _) => ChargeKind::Credit,
        (LineItemKind::Regular, Some(metric)) => ChargeKind::Usage {
            metric: metric.as_uuid(),
            dimensions: line
                .group_by_dimensions
                .iter()
                .flatten()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<BTreeMap<_, _>>(),
        },
        (LineItemKind::Regular, None) => ChargeKind::Fee,
    };
    ChargeKey {
        subscription: line.subscription_id.map(|id| id.as_uuid()),
        fee,
        from: line.start_date,
        until: line.end_date,
        kind,
    }
}

/// The line, with the id of its charge if it has none, and its sub-lines numbered under it.
pub(crate) fn identified(mut line: LineItem, lineages: &Lineages) -> LineItem {
    if line.local_id.is_empty() {
        line.local_id = charge_key(&line, lineages).line_id();
    }
    for (n, sub_line) in line.sub_lines.iter_mut().enumerate() {
        if sub_line.local_id.is_empty() {
            sub_line.local_id = format!("{}.{}", line.local_id, n + 1);
        }
    }
    line
}

/// Gives a line whose charge another line of the document already bills the key's next
/// occurrence, and renumbers its sub-lines.
pub(crate) fn disambiguate(lines: &mut [LineItem], lineages: &Lineages) {
    let mut seen = HashSet::new();
    let repeated: Vec<usize> = (0..lines.len())
        .filter(|&i| !seen.insert(lines[i].local_id.clone()))
        .collect();
    if repeated.is_empty() {
        return;
    }
    let keys: Vec<ChargeKey> = repeated
        .iter()
        .map(|&i| charge_key(&lines[i], lineages))
        .collect();
    let ids = billing_core::line_ids(&keys, seen);
    for (i, id) in repeated.into_iter().zip(ids) {
        let line = &mut lines[i];
        let previous = std::mem::replace(&mut line.local_id, id);
        for sub_line in &mut line.sub_lines {
            if let Some(n) = sub_line.local_id.strip_prefix(&format!("{previous}.")) {
                sub_line.local_id = format!("{}.{n}", line.local_id);
            }
        }
    }
}
