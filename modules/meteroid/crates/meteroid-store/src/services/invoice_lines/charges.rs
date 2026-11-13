//! Line ids from the charge a line bills (R-075).

use std::collections::{BTreeMap, HashMap, HashSet};

use billing_core::{ChargeKey, ChargeKind, Fee};
use common_domain::ids::{BaseId, SubscriptionAddOnId, SubscriptionPriceComponentId};
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

    /// The roots issued `lines` were billed under, from each line's lineage by its id.
    pub(crate) fn of_lines<'a>(
        lines: impl IntoIterator<Item = &'a LineItem>,
        line_lineage: &HashMap<String, String>,
    ) -> Self {
        Self(
            lines
                .into_iter()
                .filter_map(|line| {
                    let root = line_lineage.get(&line.local_id)?;
                    match (line.sub_component_id, line.sub_add_on_id) {
                        (Some(row), _) => Some((
                            row.as_uuid(),
                            root.parse::<SubscriptionPriceComponentId>().ok()?.as_uuid(),
                        )),
                        (None, Some(row)) => Some((
                            row.as_uuid(),
                            root.parse::<SubscriptionAddOnId>().ok()?.as_uuid(),
                        )),
                        (None, None) => None,
                    }
                })
                .collect(),
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use common_domain::ids::SubscriptionId;

    fn seats(row: SubscriptionPriceComponentId) -> LineItem {
        let day = |d| NaiveDate::from_ymd_opt(2026, 1, d).unwrap();
        LineItem {
            local_id: String::new(),
            name: "Seats".to_owned(),
            charge_type: None,
            amount_subtotal: 1_000,
            tax_rate: Default::default(),
            taxable_amount: 1_000,
            tax_amount: 0,
            amount_total: 1_000,
            tax_details: vec![],
            quantity: None,
            unit_price: None,
            start_date: day(1),
            end_date: day(31),
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: None,
            sub_component_id: Some(row),
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            group_by_dimensions: None,
            kind: LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: Some(SubscriptionId::new()),
            usage_ends_at: None,
        }
    }

    /// An issued line, read back with its lineage by id, has the key it was computed under.
    #[test]
    fn an_issued_line_reads_back_the_charge_it_billed() {
        let (row, root) = (
            SubscriptionPriceComponentId::new(),
            SubscriptionPriceComponentId::new(),
        );
        let computed = identified(
            seats(row),
            &Lineages(HashMap::from([(row.as_uuid(), root.as_uuid())])),
        );
        let lineage = HashMap::from([(computed.local_id.clone(), root.to_string())]);
        let read_back = charge_key(&computed, &Lineages::of_lines([&computed], &lineage));
        assert_eq!(read_back.line_id(), computed.local_id);
        assert_eq!(read_back.fee, Fee::Component(root.as_uuid()));
    }
}
