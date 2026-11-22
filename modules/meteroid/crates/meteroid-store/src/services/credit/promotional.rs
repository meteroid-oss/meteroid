//! Promotional credit comes off the charges after discounts and before tax, like Stripe's billing
//! credits; paid credit pays the taxed total (`value_application`). Each line records its share
//! in the figures it was quoted in, which is what the grants gave and get back. The arithmetic is
//! `billing_core::credit::settle`'s; this is the store's side of it.
use crate::StoreResult;
use crate::domain::{LineItem, TaxBreakdownItem};
use crate::repositories::credit::allocation::{CreditRequest, GrantPool};
use crate::repositories::credit::customer_credit;
use crate::repositories::invoices::{carry_over_tax_findings, compute_tax_breakdown};
use crate::services::Services;
use crate::services::credit::document::{self, settle_error};
use crate::services::credit::forecast::Drawn;
use crate::services::credit::invalid;
use crate::services::credit::scope::LineOwners;
use crate::services::credit::value_application::EstimateContext;
use crate::store::PgConn;
use billing_core::credit::Grants;
use billing_core::credit::settle as core;
use billing_core::document::{Document, Header};
use common_domain::ids::SubscriptionId;

#[cfg(test)]
use crate::domain::LineItemKind;
#[cfg(test)]
use crate::repositories::credit::allocation::Allocation;
#[cfg(test)]
use crate::repositories::credit::allocation::CreditDemand;
#[cfg(test)]
use crate::repositories::invoices::apply_tax_rates_to_priced_lines;

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

type Requested = (CreditRequest, Vec<Option<usize>>);

fn requested(
    result: Result<
        (
            billing_core::credit::allocation::Request,
            Vec<Option<usize>>,
        ),
        impl Sized,
    >,
) -> StoreResult<Requested> {
    result
        .map(|(request, of_line)| (request.into(), of_line))
        .map_err(|_| overflow())
}

/// [`pretax_request`] for an invoice's own lines, its negative lines netting the charges.
#[cfg(test)]
fn invoice_request(
    lines: &[LineItem],
    scope: LineScope<'_>,
    base: impl Fn(&LineItem) -> i64,
) -> StoreResult<Requested> {
    requested(core::invoice_request(
        &document::lines_of(lines, scope),
        |index, _| base(&lines[index]),
    ))
}

/// What promotional credit may take off `lines` before tax, each line contributing `base` (its
/// taxable amount after discounts, in quoted figures), and the demand each line falls under:
/// one per subscription, product and kind of charge.
pub(crate) fn pretax_request(
    lines: &[LineItem],
    scope: LineScope<'_>,
    base: impl Fn(&LineItem) -> i64,
) -> StoreResult<Requested> {
    requested(core::pretax_request(
        &document::lines_of(lines, scope),
        |index, _| base(&lines[index]),
    ))
}

/// Each line's share of what `allocation` covers of its demand, pro rata to its `base`.
#[cfg(test)]
fn line_shares(
    lines: &[LineItem],
    of_line: &[Option<usize>],
    allocation: &Allocation,
    base: impl Fn(&LineItem) -> i64,
) -> Vec<i64> {
    let core_lines = lines
        .iter()
        .map(|line| document::line_of(line, None))
        .collect::<Vec<_>>();
    core::line_shares(&core_lines, of_line, &allocation.into(), |index, _| {
        base(&lines[index])
    })
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
    core::taxed_base(&document::line_of(line, None), tax_inclusive)
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
    /// The document as the core settles it, its lines billing what `scope` resolves.
    pub(crate) fn core(&self, scope: LineScope<'_>) -> Document {
        Document {
            lines: document::lines_of(self.lines.as_slice(), scope),
            discount: self.discount,
            tax_inclusive: self.tax_inclusive,
            subscription: scope.subscription_id,
            header: Header {
                subtotal: *self.subtotal,
                subtotal_recurring: *self.subtotal_recurring,
                tax: *self.tax_amount,
                promotional: *self.promotional_credits,
                total: *self.total,
            },
        }
    }

    /// Moves each line to its share and re-taxes the document by its stored rates (OQ14); a
    /// provider with thresholds or a commit step must be re-called.
    #[cfg(test)]
    pub(crate) fn reprice(&mut self, shares: &[i64]) -> StoreResult<()> {
        let mut settled = self.core(LineScope {
            owners: &LineOwners::default(),
            subscription_id: None,
        });
        let repriced = settled
            .reprice(shares, &document::rates(self.lines))
            .map_err(|_| overflow())?;
        self.write(&settled, repriced.as_ref());
        Ok(())
    }

    /// Writes what the core settled: every line's total, the lines it taxed again, the breakdown
    /// rebuilt from them with what the engine found carried over, and the header.
    pub(crate) fn write(&mut self, settled: &Document, repriced: Option<&core::Repriced>) {
        for (line, settled) in self.lines.iter_mut().zip(&settled.lines) {
            line.amount_total = settled.total;
        }
        let Some(repriced) = repriced else {
            return;
        };
        for (index, netted) in repriced.retaxed.iter().zip(&repriced.netted) {
            let line = &mut self.lines[*index];
            if self.tax_inclusive {
                *line = line.clone().into_tax_inclusive_quote();
            }
            document::write_money(line, &settled.lines[*index], *netted);
        }
        let mut breakdown = compute_tax_breakdown(self.lines.as_slice());
        carry_over_tax_findings(self.tax_breakdown.as_slice(), &mut breakdown);
        *self.tax_breakdown = breakdown;
        *self.subtotal = settled.header.subtotal;
        *self.subtotal_recurring = settled.header.subtotal_recurring;
        *self.tax_amount = settled.header.tax;
        *self.total = settled.header.total;
        *self.promotional_credits = settled.header.promotional;
    }
}

impl Services {
    /// Settles the credit of taxed `lines` as finalizing at `scope.at` would draw it, their header
    /// folded from them (R-076). Returns the paid credit it would apply.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn settle_taxed_lines(
        &self,
        conn: &mut PgConn,
        scope: &EstimateContext<'_>,
        lines: &mut Vec<LineItem>,
        breakdown: &mut Vec<TaxBreakdownItem>,
        discount: i64,
        tax_inclusive: bool,
        already_paid: i64,
    ) -> StoreResult<i64> {
        let billing_core::Totals {
            mut subtotal,
            mut subtotal_recurring,
            promotional: mut promotional_credits,
            tax: mut tax_amount,
            mut total,
            ..
        } = crate::services::invoice_lines::invoice_lines::totals_of(lines, discount);
        self.settle_estimate(
            conn,
            scope,
            &mut TaxedDocument {
                lines,
                subtotal: &mut subtotal,
                subtotal_recurring: &mut subtotal_recurring,
                discount,
                tax_inclusive,
                tax_amount: &mut tax_amount,
                tax_breakdown: breakdown,
                promotional_credits: &mut promotional_credits,
                total: &mut total,
            },
            core::Cash {
                already_paid,
                ..core::Cash::none(core::TotalBasis::Folded)
            },
        )
        .await
    }

    /// Settles a taxed document's credit as finalizing at `scope.at` would draw it: its line owners,
    /// grants and kept settlements read once, then `billing_core::credit::settle`, the lines it
    /// moves written back. Returns the paid credit it would apply.
    pub(in crate::services) async fn settle_estimate(
        &self,
        conn: &mut PgConn,
        scope: &EstimateContext<'_>,
        document: &mut TaxedDocument<'_>,
        cash: core::Cash,
    ) -> StoreResult<i64> {
        let owners = LineOwners::load_tx(conn, scope.tenant_id, document.lines.as_slice()).await?;
        let settling = document.core(LineScope {
            owners: &owners,
            subscription_id: scope.subscription_id,
        });
        let rates = document::rates(document.lines.as_slice());
        // Nothing to ask of the grants reads none, as an estimate of nothing draws nothing.
        let unread = Grants {
            drawable: Vec::new(),
            settled: Vec::new(),
            at: scope.at,
        };
        let asked = core::settle(&settling, &unread, &rates, &cash).map_err(settle_error)?;
        let requests = [&asked.promotional.request, &asked.paid.request];
        let settlement = if requests.iter().any(|request| request.requested_minor > 0) {
            let grants = scope.grants_tx(conn, &requests).await?;
            core::settle(&settling, &grants, &rates, &cash).map_err(settle_error)?
        } else {
            asked
        };
        if let Some(book) = scope.book {
            for (pool, part) in [
                (GrantPool::Promotional, &settlement.promotional),
                (GrantPool::Paid, &settlement.paid),
            ] {
                book.record(Drawn {
                    denomination: customer_credit::normalized_currency(scope.currency),
                    pool,
                    fresh: part.drawn.fresh.clone().into(),
                    held: part.held.clone().into(),
                })?;
            }
        }
        document.write(&settlement.document, settlement.repriced.as_ref());
        Ok(settlement.applied)
    }
}

impl EstimateContext<'_> {
    /// What finalizing at `at` would find for `requests`: the open settlements of the periods the
    /// document bills, and the drawable grants (the forecast's book when given).
    pub(crate) async fn grants_tx(
        &self,
        conn: &mut PgConn,
        requests: &[&billing_core::credit::allocation::Request],
    ) -> StoreResult<Grants> {
        let settled = customer_credit::estimate_settled_tx(
            conn,
            self.tenant_id,
            self.currency,
            self.invoice_id,
            &self.periods,
        )
        .await?;
        let drawable = match self.book {
            Some(book) => book.candidates_tx(conn, self.currency).await?,
            None => {
                let subscriptions = requests
                    .iter()
                    .flat_map(|request| &request.demands)
                    .filter_map(|demand| demand.subscription)
                    .collect();
                customer_credit::estimate_drawable_tx(
                    conn,
                    self.tenant_id,
                    self.customer_id,
                    self.currency,
                    subscriptions,
                    self.at,
                )
                .await?
            }
        };
        Ok(Grants {
            drawable: drawable.iter().map(Into::into).collect(),
            settled: customer_credit::core_settled(&settled),
            at: self.at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Takes each share off its line, before tax.
    fn take_before_tax(lines: &mut [LineItem], shares: &[i64]) {
        for (line, share) in lines.iter_mut().zip(shares) {
            line.taxable_amount -= share;
            line.promotional_credit = (*share > 0).then_some(*share);
        }
    }

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
            usage_breakdown: None,
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
    fn shares_split_each_demand_pro_rata() {
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
        assert_eq!(lines[2].taxable_amount, 1_500);
        assert_eq!(quoted_total(&lines).unwrap(), 2_500);
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
