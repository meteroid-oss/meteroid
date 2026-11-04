//! Promotional credit comes off the charges after discounts and before tax, like Stripe's billing
//! credits; paid credit pays the taxed total (`value_application`). Each line records its share
//! in the figures it was quoted in, which is what the grants gave and get back.
use crate::StoreResult;
use crate::domain::{LineItem, LineItemKind, TaxBreakdownItem};
use crate::repositories::credit::allocation::{Allocation, CreditDemand, CreditRequest, GrantPool};
use crate::repositories::credit::customer_credit::{self, UsageCharge, UsageOwed};
use crate::repositories::invoices::{
    apply_tax_rates_to_priced_lines, carry_over_tax_findings, compute_tax_breakdown,
};
use crate::services::Services;
use crate::services::credit::invalid;
use crate::services::credit::scope::{LineOwners, line_subscription};
use crate::services::credit::value_application::EstimateContext;
use crate::services::invoice_lines::credit_units::allocate_covered;
use crate::store::PgConn;
use common_domain::ids::SubscriptionId;
use std::collections::HashMap;

fn overflow() -> error_stack::Report<crate::errors::StoreError> {
    invalid("promotional credit exceeds the supported range")
}

/// Which subscription's lines a document's demands name: each line's own (`line_subscription`).
#[derive(Clone, Copy)]
pub(crate) struct LineScope<'a> {
    pub owners: &'a LineOwners,
    /// The document's subscription; None for a consolidated invoice or a credit note's.
    pub subscription_id: Option<SubscriptionId>,
}

/// The demand a line answers to; None for a credit purchase, which credit never pays.
fn demand_of(line: &LineItem, scope: LineScope<'_>) -> Option<CreditDemand> {
    let usage = line.metric_id.is_some() && line.kind == LineItemKind::Regular;
    (line.kind != LineItemKind::Credit).then(|| CreditDemand {
        subscription_id: line_subscription(line, scope.subscription_id, scope.owners),
        product_id: line.credit_product(),
        usage,
        amount_minor: 0,
        dated_before: usage.then(|| line.usage_dated_before()),
    })
}

/// A negative line (an untaxed proration credit has no taxable amount) still nets what the
/// charges beside it may take: credit drawn on a document owing nothing would come back as paid.
fn netting_base(line: &LineItem, base: i64) -> i64 {
    if line.amount_subtotal < 0 {
        base.min(line.amount_subtotal)
    } else {
        base
    }
}

/// [`pretax_request`] for an invoice's own lines, its negative lines netting the charges.
fn invoice_request(
    lines: &[LineItem],
    scope: LineScope<'_>,
    base: impl Fn(&LineItem) -> i64,
) -> StoreResult<(CreditRequest, Vec<Option<usize>>)> {
    pretax_request(lines, scope, |line| netting_base(line, base(line)))
}

/// What promotional credit may take off `lines` before tax, each line contributing `base` (its
/// taxable amount after discounts, in quoted figures), and the demand each line falls under:
/// one per subscription, product and kind of charge.
pub(crate) fn pretax_request(
    lines: &[LineItem],
    scope: LineScope<'_>,
    base: impl Fn(&LineItem) -> i64,
) -> StoreResult<(CreditRequest, Vec<Option<usize>>)> {
    let mut demands: Vec<CreditDemand> = Vec::new();
    let mut of_line = Vec::with_capacity(lines.len());
    let mut requested = 0_i64;
    for line in lines {
        let Some(key) = demand_of(line, scope) else {
            of_line.push(None);
            continue;
        };
        let amount = base(line);
        requested = requested.checked_add(amount).ok_or_else(overflow)?;
        let index = match demands.iter().position(|demand| {
            demand.usage == key.usage
                && demand.product_id == key.product_id
                && demand.subscription_id == key.subscription_id
                && demand.dated_before == key.dated_before
        }) {
            Some(index) => index,
            None => {
                demands.push(key);
                demands.len() - 1
            }
        };
        demands[index].amount_minor = demands[index]
            .amount_minor
            .checked_add(amount)
            .ok_or_else(overflow)?;
        of_line.push(Some(index));
    }
    for demand in &mut demands {
        demand.amount_minor = demand.amount_minor.max(0);
    }
    Ok((
        CreditRequest {
            requested_minor: requested.max(0),
            demands,
        },
        of_line,
    ))
}

/// Each line's share of what `allocation` covers of its demand, pro rata to its `base`.
pub(crate) fn line_shares(
    lines: &[LineItem],
    of_line: &[Option<usize>],
    allocation: &Allocation,
    base: impl Fn(&LineItem) -> i64,
) -> Vec<i64> {
    let mut shares = vec![0_i64; lines.len()];
    for (demand, covered) in allocation.covered.iter().enumerate() {
        let members = of_line
            .iter()
            .enumerate()
            .filter(|(_, of)| **of == Some(demand))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let weights = members
            .iter()
            .map(|index| base(&lines[*index]).max(0))
            .collect::<Vec<_>>();
        for (index, share) in members
            .into_iter()
            .zip(allocate_covered(&weights, *covered))
        {
            shares[index] = share;
        }
    }
    shares
}

/// The usage charges of `request`, each line at its `base` and the rates it was taxed with: paid
/// credit that settled usage is placed on them tax included and gives back the usage it paid.
pub(crate) fn usage_charges(
    lines: &[LineItem],
    of_line: &[Option<usize>],
    request: &CreditRequest,
    tax_inclusive: bool,
    base: impl Fn(&LineItem) -> i64,
) -> Vec<UsageCharge> {
    lines
        .iter()
        .zip(of_line)
        .filter_map(|(line, demand)| {
            let demand = (*demand)?;
            request.demands.get(demand)?.usage.then(|| UsageCharge {
                demand,
                owed: UsageOwed {
                    product_id: line.product_id,
                    base_minor: base(line).max(0),
                    tax_inclusive,
                    taxes: line.tax_details.clone(),
                },
            })
        })
        .collect()
}

/// Puts back on lines priced but not yet taxed what an earlier estimate took off them.
pub(crate) fn restore_before_tax(lines: &mut [LineItem]) {
    for line in lines {
        if let Some(share) = line.promotional_credit.take() {
            line.taxable_amount += share;
        }
    }
}

/// Takes each share off its line, before tax.
pub(crate) fn take_before_tax(lines: &mut [LineItem], shares: &[i64]) {
    for (line, share) in lines.iter_mut().zip(shares) {
        line.taxable_amount -= share;
        line.promotional_credit = (*share > 0).then_some(*share);
    }
}

pub(crate) fn quoted_total(lines: &[LineItem]) -> StoreResult<i64> {
    lines
        .iter()
        .try_fold(0_i64, |sum, line| {
            sum.checked_add(line.promotional_credit.unwrap_or(0))
        })
        .ok_or_else(overflow)
}

/// The document figure of a taxed line's promotional credit (see
/// [`LineItem::net_promotional_credit`]).
pub(crate) fn net_share(line: &LineItem) -> i64 {
    line.net_promotional_credit()
}

pub(crate) fn net_total(lines: &[LineItem]) -> StoreResult<i64> {
    lines
        .iter()
        .try_fold(0_i64, |sum, line| sum.checked_add(net_share(line)))
        .ok_or_else(overflow)
}

/// A taxed line's taxable amount before its promotional credit, in the figures it was quoted in.
/// Read from its taxable amount and tax, not `amount_total`, which a draft stored by an earlier
/// release may have left at its undiscounted price.
pub(crate) fn taxed_base(line: &LineItem, tax_inclusive: bool) -> i64 {
    let share = line.promotional_credit.unwrap_or(0);
    if tax_inclusive {
        line.taxable_amount + line.tax_amount + share
    } else {
        line.taxable_amount + share
    }
}

/// The figures of a taxed document that promotional credit moves.
pub(in crate::services) struct TaxedDocument<'a> {
    pub lines: &'a mut Vec<LineItem>,
    pub subtotal: &'a mut i64,
    pub subtotal_recurring: &'a mut i64,
    pub discount: i64,
    pub tax_inclusive: bool,
    pub tax_amount: &'a mut i64,
    pub tax_breakdown: &'a mut Vec<TaxBreakdownItem>,
    pub promotional_credits: &'a mut i64,
    pub total: &'a mut i64,
}

impl TaxedDocument<'_> {
    pub(crate) fn pretax_request(
        &self,
        scope: LineScope<'_>,
    ) -> StoreResult<(CreditRequest, Vec<Option<usize>>)> {
        let tax_inclusive = self.tax_inclusive;
        invoice_request(self.lines.as_slice(), scope, |line| {
            taxed_base(line, tax_inclusive)
        })
    }

    /// One demand per line, each at most the share `shown` (by line) gave it: once cash was taken
    /// against a total, no line ends up costing less than it did, so the cash never overpays.
    pub(crate) fn pretax_request_within(
        &self,
        shown: &HashMap<&str, i64>,
        scope: LineScope<'_>,
    ) -> StoreResult<(CreditRequest, Vec<Option<usize>>)> {
        let mut demands = Vec::new();
        let mut of_line = Vec::with_capacity(self.lines.len());
        let mut net = 0_i64;
        for line in self.lines.iter() {
            let Some(demand) = demand_of(line, scope) else {
                of_line.push(None);
                continue;
            };
            let base = taxed_base(line, self.tax_inclusive);
            net = net
                .checked_add(netting_base(line, base))
                .ok_or_else(overflow)?;
            let cap = shown.get(line.local_id.as_str()).copied().unwrap_or(0);
            demands.push(CreditDemand {
                amount_minor: base.min(cap).max(0),
                ..demand
            });
            of_line.push(Some(demands.len() - 1));
        }
        let requested_minor = demands
            .iter()
            .try_fold(0_i64, |sum, demand| sum.checked_add(demand.amount_minor))
            .ok_or_else(overflow)?
            .min(net.max(0));
        Ok((
            CreditRequest {
                requested_minor,
                demands,
            },
            of_line,
        ))
    }

    pub(crate) fn usage_charges(
        &self,
        request: &CreditRequest,
        of_line: &[Option<usize>],
    ) -> Vec<UsageCharge> {
        let tax_inclusive = self.tax_inclusive;
        usage_charges(
            self.lines.as_slice(),
            of_line,
            request,
            tax_inclusive,
            |line| taxed_base(line, tax_inclusive),
        )
    }

    pub(crate) fn shares(&self, of_line: &[Option<usize>], allocation: &Allocation) -> Vec<i64> {
        let tax_inclusive = self.tax_inclusive;
        line_shares(self.lines.as_slice(), of_line, allocation, |line| {
            taxed_base(line, tax_inclusive)
        })
    }

    /// Moves each line to its share and re-taxes the document by its stored rates, each rate
    /// rounded once over its lines (OQ14): exact for the built-in resolvers (lookups); a provider
    /// with thresholds or a commit step must be re-called. The total is what the lines add up to;
    /// the discount stays, promotional credit is the rest.
    pub(crate) fn reprice(&mut self, shares: &[i64]) -> StoreResult<()> {
        // A draft stored by an earlier release may carry an untaxed line's undiscounted price.
        for line in self.lines.iter_mut() {
            line.amount_total = line.taxable_amount + line.tax_amount;
        }
        let share_of = |index: usize, line: &LineItem| {
            shares
                .get(index)
                .copied()
                .unwrap_or_else(|| line.promotional_credit.unwrap_or(0))
        };
        let moved = self
            .lines
            .iter()
            .enumerate()
            .any(|(index, line)| line.promotional_credit.unwrap_or(0) != share_of(index, line));
        if !moved {
            return Ok(());
        }
        let retaxed: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .filter(|(index, line)| {
                line.promotional_credit.unwrap_or(0) != share_of(*index, line)
                    || (!line.tax_details.is_empty()
                        && !crate::services::credit::prepaid_purchase_tax::taxed_at_redemption(
                            line,
                        ))
            })
            .map(|(index, _)| index)
            .collect();
        let mut priced: Vec<LineItem> = retaxed
            .iter()
            .map(|index| {
                let line = &self.lines[*index];
                let share = share_of(*index, line);
                let base = taxed_base(line, self.tax_inclusive);
                let mut priced = if self.tax_inclusive {
                    line.clone().into_tax_inclusive_quote()
                } else {
                    line.clone()
                };
                priced.taxable_amount = base - share;
                priced.promotional_credit = (share > 0).then_some(share);
                priced
            })
            .collect();
        apply_tax_rates_to_priced_lines(&mut priced, self.tax_inclusive);
        for (index, line) in retaxed.into_iter().zip(priced) {
            self.lines[index] = line;
        }
        let mut breakdown = compute_tax_breakdown(self.lines.as_slice());
        carry_over_tax_findings(self.tax_breakdown.as_slice(), &mut breakdown);
        *self.tax_breakdown = breakdown;
        *self.tax_amount = self
            .lines
            .iter()
            .try_fold(0_i64, |sum, line| sum.checked_add(line.tax_amount))
            .ok_or_else(overflow)?;
        let sum = |lines: &[LineItem], recurring: bool| {
            lines
                .iter()
                .filter(|line| !recurring || line.metric_id.is_none())
                .try_fold(0_i64, |sum, line| sum.checked_add(line.amount_subtotal))
                .ok_or_else(overflow)
        };
        *self.subtotal = sum(self.lines.as_slice(), false)?;
        *self.subtotal_recurring = sum(self.lines.as_slice(), true)?;
        *self.total = self
            .lines
            .iter()
            .try_fold(0_i64, |sum, line| sum.checked_add(line.amount_total))
            .ok_or_else(overflow)?;
        *self.promotional_credits = self
            .subtotal
            .checked_sub(self.discount)
            .and_then(|left| left.checked_add(*self.tax_amount))
            .and_then(|left| left.checked_sub(*self.total))
            .ok_or_else(overflow)?;
        Ok(())
    }
}

impl Services {
    /// Takes off lines priced but not yet taxed the promotional credit finalizing at `scope.at`
    /// would draw against them.
    pub(in crate::services) async fn take_promotional_estimate(
        &self,
        conn: &mut PgConn,
        scope: &EstimateContext<'_>,
        lines: &mut [LineItem],
    ) -> StoreResult<()> {
        restore_before_tax(lines);
        let base = |line: &LineItem| line.taxable_amount;
        let owners = LineOwners::load_tx(conn, scope.tenant_id, lines).await?;
        let line_scope = LineScope {
            owners: &owners,
            subscription_id: scope.subscription_id,
        };
        let (request, of_line) = invoice_request(lines, line_scope, base)?;
        // Not taxed yet: paid settlements are placed at face value until the taxed lines are
        // estimated again (`reprice_promotional_estimate`).
        let mut charges = usage_charges(lines, &of_line, &request, false, base);
        for charge in &mut charges {
            charge.owed.taxes.clear();
        }
        let allocation = scope
            .estimate(conn, GrantPool::Promotional, &request, &charges)
            .await?;
        let shares = line_shares(lines, &of_line, &allocation, base);
        take_before_tax(lines, &shares);
        Ok(())
    }

    /// [`Services::take_promotional_estimate`] for lines already taxed (kept, merged or edited):
    /// re-taxes only the lines whose share moved.
    pub(in crate::services) async fn reprice_promotional_estimate(
        &self,
        conn: &mut PgConn,
        scope: &EstimateContext<'_>,
        document: &mut TaxedDocument<'_>,
    ) -> StoreResult<()> {
        let owners = LineOwners::load_tx(conn, scope.tenant_id, document.lines.as_slice()).await?;
        let (request, of_line) = document.pretax_request(LineScope {
            owners: &owners,
            subscription_id: scope.subscription_id,
        })?;
        let charges = document.usage_charges(&request, &of_line);
        let allocation = scope
            .estimate(conn, GrantPool::Promotional, &request, &charges)
            .await?;
        let shares = document.shares(&of_line, &allocation);
        document.reprice(&shares)
    }
}

impl EstimateContext<'_> {
    pub(crate) async fn estimate(
        &self,
        conn: &mut PgConn,
        pool: GrantPool,
        request: &CreditRequest,
        charges: &[UsageCharge],
    ) -> StoreResult<Allocation> {
        if let Some(book) = self.book {
            return crate::services::credit::forecast::estimate_on_book(
                conn, book, self, pool, request, charges,
            )
            .await;
        }
        customer_credit::estimate_tx(
            conn,
            self.tenant_id,
            self.customer_id,
            self.currency,
            self.invoice_id,
            &self.periods,
            pool,
            request,
            charges,
            self.at,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_scope(owners: &LineOwners) -> LineScope<'_> {
        LineScope {
            owners,
            subscription_id: None,
        }
    }
    use crate::domain::TaxInclusiveQuote;
    use chrono::NaiveDate;
    use common_domain::ids::{BaseId, BillableMetricId, CreditGrantId};
    use rust_decimal::Decimal;

    fn line(amount: i64, usage: bool, kind: LineItemKind) -> LineItem {
        let day = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        LineItem {
            local_id: format!("l{amount}"),
            name: "line".to_owned(),
            charge_type: None,
            amount_subtotal: amount,
            tax_rate: Decimal::ZERO,
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
            metric_id: usage.then(BillableMetricId::new),
            description: None,
            group_by_dimensions: None,
            kind,
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

    #[test]
    fn credit_purchases_are_never_eligible_and_usage_is_its_own_demand() {
        let lines = [
            line(3_000, false, LineItemKind::Regular),
            line(1_000, true, LineItemKind::Regular),
            line(5_000, false, LineItemKind::Credit),
            line(500, false, LineItemKind::MinimumShortfall),
        ];
        let (request, of_line) = pretax_request(&lines, no_scope(&LineOwners::default()), |l| {
            l.taxable_amount
        })
        .unwrap();
        assert_eq!(request.requested_minor, 4_500);
        assert_eq!(
            request.demands,
            vec![
                CreditDemand {
                    subscription_id: None,
                    product_id: None,
                    usage: false,
                    amount_minor: 3_500,
                    dated_before: None,
                },
                CreditDemand {
                    subscription_id: None,
                    product_id: None,
                    usage: true,
                    amount_minor: 1_000,
                    dated_before: Some(lines[1].usage_dated_before()),
                },
            ]
        );
        assert_eq!(of_line, vec![Some(0), Some(1), None, Some(0)]);
    }

    /// Proration credits are untaxed (no taxable amount) yet net the charges beside them.
    fn untaxed_credit(amount: i64) -> LineItem {
        LineItem {
            taxable_amount: 0,
            ..line(amount, false, LineItemKind::Regular)
        }
    }

    #[test]
    fn a_document_owing_nothing_requests_no_promotional_credit() {
        let downgrade = [
            untaxed_credit(-5_110),
            untaxed_credit(-1_290),
            line(1_497, false, LineItemKind::Regular),
            line(516, false, LineItemKind::Regular),
        ];
        let (request, _) = invoice_request(&downgrade, no_scope(&LineOwners::default()), |l| {
            l.taxable_amount
        })
        .unwrap();
        assert_eq!(request.requested_minor, 0);
        assert_eq!(request.demands[0].amount_minor, 0);

        let upgrade = [
            untaxed_credit(-1_497),
            untaxed_credit(-516),
            line(5_110, false, LineItemKind::Regular),
            line(1_290, false, LineItemKind::Regular),
        ];
        let (request, of_line) = invoice_request(&upgrade, no_scope(&LineOwners::default()), |l| {
            l.taxable_amount
        })
        .unwrap();
        assert_eq!(
            request.requested_minor, 4_387,
            "the net, not the 6,400 charged"
        );
        let allocation = Allocation {
            draws: vec![(CreditGrantId::new(), 4_387)],
            covered: vec![4_387],
            placed: Vec::new(),
        };
        assert_eq!(
            line_shares(&upgrade, &of_line, &allocation, |l| l.taxable_amount),
            vec![0, 0, 3_503, 884],
            "only the charges carry it"
        );
    }

    #[test]
    fn shares_split_each_demand_pro_rata_and_restore_exactly() {
        let mut lines = vec![
            line(3_000, false, LineItemKind::Regular),
            line(1_000, false, LineItemKind::Regular),
            line(2_000, true, LineItemKind::Regular),
        ];
        let base = |l: &LineItem| l.taxable_amount;
        let (_, of_line) = pretax_request(&lines, no_scope(&LineOwners::default()), base).unwrap();
        let allocation = Allocation {
            draws: vec![(CreditGrantId::new(), 2_500)],
            covered: vec![2_000, 500],
            placed: Vec::new(),
        };
        let shares = line_shares(&lines, &of_line, &allocation, base);
        assert_eq!(shares, vec![1_500, 500, 500]);
        take_before_tax(&mut lines, &shares);
        assert_eq!(lines[0].taxable_amount, 1_500);
        assert_eq!(quoted_total(&lines).unwrap(), 2_500);
        restore_before_tax(&mut lines);
        assert_eq!(lines[2].taxable_amount, 2_000);
        assert_eq!(lines[2].promotional_credit, None);
    }

    fn rated(mut item: LineItem, rate: Decimal) -> LineItem {
        item.tax_rate = rate;
        item.tax_details = vec![crate::domain::TaxDetail {
            tax_rate: rate,
            tax_name: "VAT".to_owned(),
            tax_amount: 0,
        }];
        item
    }

    fn reprice(lines: &mut Vec<LineItem>, tax_inclusive: bool, shares: &[i64]) -> [i64; 5] {
        let mut subtotal = lines.iter().map(|l| l.amount_subtotal).sum::<i64>();
        let mut tax = lines.iter().map(|l| l.tax_amount).sum::<i64>();
        let mut total = lines.iter().map(|l| l.amount_total).sum::<i64>();
        let mut promotional = net_total(lines).unwrap();
        let discount = subtotal - promotional + tax - total;
        let (mut recurring, mut breakdown) = (0, Vec::new());
        TaxedDocument {
            lines,
            subtotal: &mut subtotal,
            subtotal_recurring: &mut recurring,
            discount,
            tax_inclusive,
            tax_amount: &mut tax,
            tax_breakdown: &mut breakdown,
            promotional_credits: &mut promotional,
            total: &mut total,
        }
        .reprice(shares)
        .unwrap();
        [subtotal, discount, promotional, tax, total]
    }

    /// 9.99 quoted at 20% with a 1.00 coupon: the promotional share moving from 3.33 to 0.10
    /// leaves 8.89 to pay, to the cent, on the line and on the invoice.
    #[test]
    fn a_moving_share_on_a_discounted_tax_inclusive_line_keeps_every_cent() {
        let mut quoted = rated(line(999, false, LineItemKind::Regular), Decimal::new(2, 1));
        quoted.taxable_amount = 999 - 100 - 333;
        quoted.promotional_credit = Some(333);
        apply_tax_rates_to_priced_lines(std::slice::from_mut(&mut quoted), true);
        let mut lines = vec![quoted];
        let [subtotal, discount, promotional, tax, total] = reprice(&mut lines, true, &[10]);
        assert_eq!(lines[0].amount_total, 889);
        assert_eq!(total, 889);
        assert_eq!(subtotal - discount - promotional + tax, total);
        assert_eq!(lines[0].promotional_credit, Some(10));
    }

    /// Promotional credit moving from a 20% line to a 5.5% one re-taxes both by their rates.
    #[test]
    fn a_share_moving_between_rates_re_taxes_both_lines() {
        let mut standard = rated(
            line(10_000, false, LineItemKind::Regular),
            Decimal::new(20, 2),
        );
        standard.taxable_amount = 7_000;
        standard.promotional_credit = Some(3_000);
        standard.tax_amount = 1_400;
        standard.amount_total = 8_400;
        let mut reduced = rated(
            line(10_000, false, LineItemKind::Regular),
            Decimal::new(55, 3),
        );
        reduced.local_id = "reduced".to_owned();
        reduced.tax_amount = 550;
        reduced.amount_total = 10_550;
        let mut lines = vec![standard, reduced];
        let [_, _, promotional, tax, total] = reprice(&mut lines, false, &[0, 3_000]);
        assert_eq!(lines[0].tax_amount, 2_000);
        assert_eq!(lines[1].tax_amount, 385);
        assert_eq!((promotional, tax, total), (3_000, 2_385, 19_385));
    }

    /// Promotional credit moving 0.01 from a 20% line to a 5.5% one: 20% is rounded once over its
    /// two lines (0.13 taxed 0.03, the extra cent on the larger remainder), not line by line,
    /// which left 0.02 (OQ14).
    #[test]
    fn a_share_moving_across_rates_rounds_each_rate_once_over_its_lines() {
        let taxed = |amount: i64, rate: Decimal, taxable: i64, tax: i64| {
            let mut item = rated(line(amount, false, LineItemKind::Regular), rate);
            item.taxable_amount = taxable;
            item.promotional_credit = (amount > taxable).then_some(amount - taxable);
            item.tax_amount = tax;
            item.tax_details[0].tax_amount = tax;
            item.amount_total = taxable + tax;
            item
        };
        let mut lines = vec![
            taxed(10, Decimal::new(20, 2), 5, 1),
            taxed(7, Decimal::new(20, 2), 7, 1),
            taxed(1_000, Decimal::new(55, 3), 1_000, 55),
        ];
        let [_, _, promotional, tax, total] = reprice(&mut lines, false, &[4, 0, 1]);
        assert_eq!(
            lines
                .iter()
                .map(|line| (line.taxable_amount, line.tax_amount))
                .collect::<Vec<_>>(),
            vec![(6, 1), (7, 2), (999, 55)]
        );
        assert_eq!((promotional, tax, total), (5, 58, 1_070));
    }

    /// A line the promotional credit took whole keeps its rate, so giving the credit back taxes it.
    #[test]
    fn a_line_taken_whole_is_taxed_again_when_the_credit_is_gone() {
        let mut taken = rated(
            line(3_000, false, LineItemKind::Regular),
            Decimal::new(20, 2),
        );
        taken.taxable_amount = 0;
        taken.amount_total = 0;
        taken.promotional_credit = Some(3_000);
        let mut lines = vec![taken];
        let [_, _, promotional, tax, total] = reprice(&mut lines, false, &[0]);
        assert_eq!((promotional, tax, total), (0, 600, 3_600));
    }

    #[test]
    fn a_tax_inclusive_share_reads_net_on_the_document() {
        // 120.00 quoted at 20%, 50.00 of promotional credit: 70.00 left, 58.33 net + 11.67 tax.
        let mut taxed = line(10_000, false, LineItemKind::Regular);
        taxed.taxable_amount = 5_833;
        taxed.tax_amount = 1_167;
        taxed.amount_total = 7_000;
        taxed.promotional_credit = Some(5_000);
        taxed.tax_inclusive_quote = Some(TaxInclusiveQuote {
            amount_subtotal: 12_000,
            unit_price: None,
            sub_lines: Vec::new(),
        });
        assert_eq!(net_share(&taxed), 4_167);
        assert_eq!(taxed_base(&taxed, true), 12_000);
    }
}
