//! Where paid credit sits on a document's lines: each grant's share of each demand the allocator
//! placed it on, spread over that demand's lines as `promotional::line_shares` spreads coverage.
//! Revenue facts recognize credit on the lines it paid, not pro rata over the invoice.
use crate::StoreResult;
use crate::domain::LineItem;
use crate::repositories::credit::allocation::{Allocation, CreditRequest};
use crate::services::invoice_lines::credit_units::allocate_covered;
use crate::store::PgConn;
use common_domain::ids::{CreditGrantId, CreditNoteId, InvoiceId, TenantId};
use diesel_models::customer_credit::CreditLineAllocationRowNew;
use std::collections::BTreeMap;

/// Per grant, what it paid of each line (by local id), and of no line under `None`.
pub(crate) fn line_allocation(
    lines: &[LineItem],
    of_line: &[Option<usize>],
    request: &CreditRequest,
    allocation: &Allocation,
    base: impl Fn(&LineItem) -> i64,
) -> Vec<(CreditGrantId, Option<String>, i64)> {
    let mut out = Vec::new();
    for (grant_id, columns) in &allocation.placed {
        let mut placed: BTreeMap<Option<String>, i64> = BTreeMap::new();
        for (demand, amount) in columns.iter().enumerate() {
            if *amount <= 0 {
                continue;
            }
            if demand >= request.demands.len() {
                *placed.entry(None).or_default() += amount;
                continue;
            }
            let members: Vec<usize> = of_line
                .iter()
                .enumerate()
                .filter(|(_, of)| **of == Some(demand))
                .map(|(index, _)| index)
                .collect();
            let weights: Vec<i64> = members
                .iter()
                .map(|index| base(&lines[*index]).max(0))
                .collect();
            let shares = allocate_covered(&weights, *amount);
            let mut left = *amount;
            for (index, share) in members.into_iter().zip(shares) {
                if share > 0 {
                    *placed
                        .entry(Some(lines[index].local_id.clone()))
                        .or_default() += share;
                    left -= share;
                }
            }
            if left > 0 {
                *placed.entry(None).or_default() += left;
            }
        }
        out.extend(
            placed
                .into_iter()
                .filter(|(_, amount)| *amount > 0)
                .map(|(line, amount)| (*grant_id, line, amount)),
        );
    }
    out
}

/// Records [`line_allocation`] for `invoice_id` (or the note giving credit back on it). The caller
/// holds the customer's lock.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_line_allocation_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    credit_note_id: Option<CreditNoteId>,
    lines: &[LineItem],
    of_line: &[Option<usize>],
    request: &CreditRequest,
    allocation: &Allocation,
    base: impl Fn(&LineItem) -> i64,
) -> StoreResult<()> {
    let rows: Vec<CreditLineAllocationRowNew> =
        line_allocation(lines, of_line, request, allocation, base)
            .into_iter()
            .map(
                |(grant_id, line_local_id, amount)| CreditLineAllocationRowNew {
                    id: uuid::Uuid::now_v7(),
                    tenant_id,
                    invoice_id,
                    credit_note_id,
                    grant_id,
                    line_local_id,
                    amount,
                },
            )
            .collect();
    CreditLineAllocationRowNew::record(conn, tenant_id, invoice_id, credit_note_id, &rows)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LineItemKind;
    use crate::repositories::credit::allocation::CreditDemand;
    use common_domain::ids::BaseId;

    fn line(local_id: &str, amount: i64) -> LineItem {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        LineItem {
            local_id: local_id.to_owned(),
            name: local_id.to_owned(),
            charge_type: None,
            amount_subtotal: amount,
            tax_rate: rust_decimal::Decimal::ZERO,
            taxable_amount: amount,
            tax_amount: 0,
            amount_total: amount,
            tax_details: Vec::new(),
            quantity: None,
            unit_price: None,
            start_date: day,
            end_date: day,
            sub_lines: Vec::new(),
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
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
            subscription_id: None,
            usage_ends_at: None,
        }
    }

    fn demand(usage: bool, amount_minor: i64) -> CreditDemand {
        CreditDemand {
            subscription_id: None,
            product_id: None,
            usage,
            amount_minor,
            dated_before: None,
        }
    }

    /// Usage credit paid the two usage lines, customer-wide credit the fee and the cent no line
    /// names: each grant lands on the lines of the demands it paid, and the cent on none.
    #[test]
    fn each_grant_sits_on_the_lines_of_the_demands_it_paid() {
        let lines = [line("a", 3_000), line("fee", 4_900), line("b", 1_000)];
        let of_line = [Some(0), Some(1), Some(0)];
        let request = CreditRequest {
            requested_minor: 8_901,
            demands: vec![demand(true, 4_000), demand(false, 4_900)],
        };
        let (usage, wide) = (CreditGrantId::new(), CreditGrantId::new());
        let allocation = Allocation {
            draws: vec![(usage, 3_001), (wide, 4_901)],
            covered: vec![3_001, 4_900],
            placed: vec![(usage, vec![3_001, 0, 0]), (wide, vec![0, 4_900, 1])],
        };
        let placed = line_allocation(&lines, &of_line, &request, &allocation, |l| l.amount_total);
        let expected = vec![
            (usage, Some("a".to_owned()), 2_251),
            (usage, Some("b".to_owned()), 750),
            (wide, None, 1),
            (wide, Some("fee".to_owned()), 4_900),
        ];
        assert_eq!(placed, expected);
    }
}
