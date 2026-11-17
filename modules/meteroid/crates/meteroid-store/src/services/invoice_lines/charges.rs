//! Line ids from the charge a line bills (R-075).

use std::collections::HashMap;

use billing_core::ChargeKey;
use billing_core::assembly::charges as core;
use common_domain::ids::{BaseId, SubscriptionAddOnId, SubscriptionPriceComponentId};

use crate::domain::{LineItem, LineItemKind, SubscriptionDetails};

/// The lineage root of each component and add-on of a subscription, by row id.
pub(crate) struct Lineages(core::Lineages);

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
        Self(core::Lineages(components.chain(add_ons).collect()))
    }

    /// From each row's lineage root, closed rows' included.
    pub(crate) fn of_roots(roots: impl IntoIterator<Item = (uuid::Uuid, uuid::Uuid)>) -> Self {
        Self(core::Lineages(roots.into_iter().collect()))
    }

    pub(crate) fn none() -> Self {
        Self(core::Lineages::default())
    }

    /// The roots issued `lines` were billed under, from each line's lineage by its id.
    pub(crate) fn of_lines<'a>(
        lines: impl IntoIterator<Item = &'a LineItem>,
        line_lineage: &HashMap<String, String>,
    ) -> Self {
        Self(core::Lineages(
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
        ))
    }
}

fn charged_line(line: &LineItem) -> core::ChargedLine {
    core::ChargedLine {
        kind: match line.kind {
            LineItemKind::Regular => core::LineKind::Regular,
            LineItemKind::Credit => core::LineKind::Credit,
            LineItemKind::MinimumShortfall => core::LineKind::MinimumShortfall,
        },
        subscription: line.subscription_id,
        component: line.sub_component_id,
        add_on: line.sub_add_on_id,
        product: line.product_id,
        metric: line.metric_id,
        dimensions: line
            .group_by_dimensions
            .iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        from: line.start_date,
        until: line.end_date,
        // Keyed in money until billing reads charges from the core: a credit-unit key would
        // change the ids of today's credit-priced usage lines.
        credit_unit: None,
    }
}

pub(crate) fn charge_key(line: &LineItem, lineages: &Lineages) -> ChargeKey {
    core::charge_key(&charged_line(line), &lineages.0)
}

/// The charge a line bills as `evaluate` keys it: one priced in a credit unit in that unit. Its
/// id stays [`charge_key`]'s (R-075).
pub(crate) fn evaluated_charge_key(line: &LineItem, lineages: &Lineages) -> ChargeKey {
    let mut charged = charged_line(line);
    charged.credit_unit = line.credit_unit.as_ref().map(|unit| unit.unit_id);
    core::charge_key(&charged, &lineages.0)
}

/// The line, with the id of its charge if it has none, and its sub-lines numbered under it.
pub(crate) fn identified(mut line: LineItem, lineages: &Lineages) -> LineItem {
    if line.local_id.is_empty() {
        line.local_id = charge_key(&line, lineages).line_id();
    }
    for (n, sub_line) in line.sub_lines.iter_mut().enumerate() {
        if sub_line.local_id.is_empty() {
            sub_line.local_id = core::sub_line_id(&line.local_id, n);
        }
    }
    line
}

/// Gives a line whose charge another line of the document already bills the key's next
/// occurrence, and renumbers its sub-lines.
pub(crate) fn disambiguate(lines: &mut [LineItem], lineages: &Lineages) {
    let ids: Vec<&str> = lines.iter().map(|line| line.local_id.as_str()).collect();
    let renamed = core::disambiguate(&ids, |i| charge_key(&lines[i], lineages));
    for (i, id) in renamed {
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
    use billing_core::Fee;
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
            usage_breakdown: None,
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
            &Lineages(core::Lineages(HashMap::from([(
                row.as_uuid(),
                root.as_uuid(),
            )]))),
        );
        let lineage = HashMap::from([(computed.local_id.clone(), root.to_string())]);
        let read_back = charge_key(&computed, &Lineages::of_lines([&computed], &lineage));
        assert_eq!(read_back.line_id(), computed.local_id);
        assert_eq!(read_back.fee, Fee::Component(root));
    }
}
