//! The customer credit an invoice may draw, and how the draw lands on its totals. Promotional
//! credit comes off the lines before tax (`promotional`); paid credit pays the taxed total.
use crate::StoreResult;
use crate::domain::{InvoiceLinesPatch, LineItem, LineItemKind};
use crate::errors::StoreError;
use crate::repositories::credit::allocation::{CreditDemand, CreditRequest, GrantPool};
use crate::services::Services;
use crate::services::credit::forecast::CreditBooks;
use crate::services::credit::invalid;
use crate::services::credit::scope::{LineOwners, line_subscription};
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::payments::PaymentTransactionRow;
use error_stack::Report;

/// Whose credit an invoice draws and which usage periods it bills, for an estimate of the draw
/// finalizing at `at` would make.
#[derive(Debug, Clone)]
pub(crate) struct EstimateContext<'a> {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub currency: &'a str,
    pub invoice_id: Option<InvoiceId>,
    /// The invoice's subscription; None on a consolidated invoice, whose lines name theirs.
    pub subscription_id: Option<SubscriptionId>,
    pub periods: Vec<(SubscriptionId, NaiveDate)>,
    pub at: DateTime<Utc>,
    /// A forecast's grants, already spent by the documents billed before this one; None reads
    /// the ledger.
    pub book: Option<&'a CreditBooks>,
}

/// The usage periods `lines` of a subscription's invoice bill, whose settlements it takes over.
pub(crate) fn billed_periods(
    subscription_id: Option<SubscriptionId>,
    lines: &[LineItem],
) -> Vec<(SubscriptionId, NaiveDate)> {
    let Some(subscription_id) = subscription_id else {
        return Vec::new();
    };
    lines
        .iter()
        .filter(|line| line.metric_id.is_some())
        .map(|line| (subscription_id, line.start_date))
        .collect()
}

/// A line's charge as the customer was quoted it, tax included on a tax-inclusive invoice.
pub fn quoted_subtotal(line: &LineItem, tax_inclusive: bool) -> i64 {
    if tax_inclusive || line.tax_inclusive_quote.is_some() {
        line.clone().into_tax_inclusive_quote().amount_subtotal
    } else {
        line.amount_subtotal
    }
}

/// Gross of the usage charges, read in the figures the customer was quoted (tax included on a
/// tax-inclusive invoice), as the invoice total and the live usage estimate are.
pub fn usage_gross_subtotal(lines: &[LineItem], tax_inclusive: bool) -> StoreResult<i64> {
    lines
        .iter()
        .filter(|line| line.metric_id.is_some() && line.kind == LineItemKind::Regular)
        .try_fold(0_i64, |total, line| {
            total.checked_add(quoted_subtotal(line, tax_inclusive))
        })
        .ok_or_else(|| invalid("invoice usage subtotal exceeds the supported range"))
}

/// Cash already committed to an invoice (pending, ready or settled charges); failed or cancelled
/// attempts and refunds do not count.
pub(crate) async fn committed_cash_minor(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> StoreResult<i64> {
    let transactions = PaymentTransactionRow::list_by_invoice_id(conn, invoice_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    transactions
        .iter()
        .filter(|tx| tx.transaction.payment_type != PaymentTypeEnum::Refund)
        .filter(|tx| {
            matches!(
                tx.transaction.status,
                PaymentStatusEnum::Pending | PaymentStatusEnum::Ready | PaymentStatusEnum::Settled
            )
        })
        .try_fold(0_i64, |total, tx| total.checked_add(tx.transaction.amount))
        .ok_or_else(|| invalid("invoice committed cash exceeds the supported range"))
}

/// The unpaid part of an invoice paid credit may cover, and per subscription, product and kind of
/// charge what its lines still cost (after discount and promotional credit, tax included): usage
/// credit pays the usage, credit of a subscription its lines, credit restricted to products
/// theirs. Credit never pays for a credit purchase.
pub(crate) fn credit_request(
    lines: &[LineItem],
    owners: &LineOwners,
    invoice_subscription: Option<SubscriptionId>,
    total: i64,
    already_paid: i64,
) -> StoreResult<CreditRequest> {
    credit_request_of_lines(lines, owners, invoice_subscription, total, already_paid)
        .map(|(request, _)| request)
}

/// [`credit_request`], with the demand each line falls under (None for a credit purchase).
pub(crate) fn credit_request_of_lines(
    lines: &[LineItem],
    owners: &LineOwners,
    invoice_subscription: Option<SubscriptionId>,
    total: i64,
    already_paid: i64,
) -> StoreResult<(CreditRequest, Vec<Option<usize>>)> {
    let overflow = || invalid("invoice credit request exceeds the supported range");
    let credit_purchase = lines
        .iter()
        .filter(|line| line.kind == LineItemKind::Credit)
        .try_fold(0_i64, |sum, line| sum.checked_add(line.amount_total.max(0)))
        .ok_or_else(|| invalid("invoice credit purchase total exceeds the supported range"))?;
    let mut demands: Vec<CreditDemand> = Vec::new();
    let mut of_line = Vec::with_capacity(lines.len());
    for line in lines {
        if line.kind == LineItemKind::Credit {
            of_line.push(None);
            continue;
        }
        let usage = line.metric_id.is_some() && line.kind == LineItemKind::Regular;
        let subscription_id = line_subscription(line, invoice_subscription, owners);
        let dated_before = usage.then(|| line.usage_dated_before());
        let amount = line.amount_total;
        match demands.iter().position(|demand| {
            demand.usage == usage
                && demand.product_id == line.credit_product()
                && demand.subscription_id == subscription_id
                && demand.dated_before == dated_before
        }) {
            Some(index) => {
                demands[index].amount_minor = demands[index]
                    .amount_minor
                    .checked_add(amount)
                    .ok_or_else(overflow)?;
                of_line.push(Some(index));
            }
            None => {
                demands.push(CreditDemand {
                    subscription_id,
                    product_id: line.credit_product(),
                    usage,
                    amount_minor: amount,
                    dated_before,
                });
                of_line.push(Some(demands.len() - 1));
            }
        }
    }
    for demand in &mut demands {
        demand.amount_minor = demand.amount_minor.max(0);
    }
    Ok((
        CreditRequest {
            requested_minor: total
                .saturating_sub(already_paid.max(0))
                .min(total.saturating_sub(credit_purchase))
                .max(0),
            demands,
        },
        of_line,
    ))
}

/// Lands credit the ledger applied on the invoice totals, after anything already paid.
pub(crate) fn record_applied_credit(
    patch: &mut InvoiceLinesPatch,
    applied_minor: i64,
) -> StoreResult<()> {
    if applied_minor <= 0 {
        return Ok(());
    }
    let already_paid = patch
        .total
        .checked_sub(patch.amount_due)
        .and_then(|amount| amount.checked_sub(patch.applied_credits))
        .unwrap_or(0)
        .max(0);
    patch.applied_credits = patch
        .applied_credits
        .checked_add(applied_minor)
        .ok_or_else(|| invalid("invoice applied credit exceeds the supported range"))?;
    let maximum_credit = patch.total.saturating_sub(already_paid).max(0);
    if patch.applied_credits > maximum_credit {
        return Err(invalid("invoice applied credit exceeds the unpaid total"));
    }
    patch.amount_due = patch
        .total
        .saturating_sub(already_paid)
        .saturating_sub(patch.applied_credits)
        .max(0);
    Ok(())
}

impl Services {
    /// Draft `applied_credits` preview: the paid credit this invoice would draw on its taxed total
    /// if finalized at `scope.at`.
    pub(in crate::services) async fn preview_credit_for_lines(
        &self,
        conn: &mut PgConn,
        scope: &EstimateContext<'_>,
        lines: &[LineItem],
        total: i64,
        already_paid: i64,
    ) -> StoreResult<i64> {
        let owners = LineOwners::load_tx(conn, scope.tenant_id, lines).await?;
        scope
            .estimate(
                conn,
                GrantPool::Paid,
                &credit_request(lines, &owners, scope.subscription_id, total, already_paid)?,
                &[],
            )
            .await?
            .total()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::invoice_lines::LineItem;
    use chrono::NaiveDate;
    use common_domain::ids::{BaseId, BillableMetricId, SubscriptionId};
    use rust_decimal::Decimal;

    fn line(amount: i64, usage: bool, kind: LineItemKind) -> LineItem {
        let day = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        LineItem {
            local_id: "l".to_owned(),
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

    fn usage_of(request: &CreditRequest) -> i64 {
        request
            .demands
            .iter()
            .filter(|demand| demand.usage)
            .map(|demand| demand.amount_minor)
            .sum()
    }

    #[test]
    fn credit_never_pays_for_a_credit_purchase() {
        let lines = [
            line(3_000, false, LineItemKind::Regular),
            line(5_000, false, LineItemKind::Credit),
        ];
        let request = credit_request(&lines, &LineOwners::default(), None, 8_000, 0).unwrap();
        assert_eq!(request.requested_minor, 3_000);
        assert_eq!(usage_of(&request), 0);
    }

    #[test]
    fn usage_credit_is_capped_at_the_usage_gross() {
        let lines = [
            line(1_000, true, LineItemKind::Regular),
            line(4_000, false, LineItemKind::Regular),
        ];
        let request = credit_request(&lines, &LineOwners::default(), None, 5_000, 500).unwrap();
        assert_eq!(request.requested_minor, 4_500);
        assert_eq!(usage_of(&request), 1_000);
        assert_eq!(
            credit_request(&lines, &LineOwners::default(), None, -100, 0)
                .unwrap()
                .requested_minor,
            0
        );
    }

    #[test]
    fn applied_credit_comes_after_what_was_paid() {
        let mut patch = InvoiceLinesPatch {
            line_items: Vec::new(),
            amount_due: 700,
            subtotal: 1_000,
            subtotal_recurring: 1_000,
            total: 1_000,
            tax_amount: 0,
            applied_credits: 0,
            promotional_credits: 0,
            applied_coupons: Vec::new(),
            tax_breakdown: Vec::new(),
            annexes: Vec::new(),
        };
        record_applied_credit(&mut patch, 400).unwrap();
        assert_eq!((patch.applied_credits, patch.amount_due), (400, 300));
        assert!(record_applied_credit(&mut patch, 301).is_err());
    }

    #[test]
    fn a_tax_inclusive_usage_cap_is_read_in_the_quoted_figures() {
        let mut taxed = line(1_000, true, LineItemKind::Regular);
        taxed.tax_rate = Decimal::new(2, 1);
        assert_eq!(
            usage_gross_subtotal(&[taxed.clone()], false).unwrap(),
            1_000
        );
        assert_eq!(usage_gross_subtotal(&[taxed.clone()], true).unwrap(), 1_200);

        taxed.tax_inclusive_quote = Some(crate::domain::invoice_lines::TaxInclusiveQuote {
            amount_subtotal: 1_199,
            unit_price: None,
            sub_lines: Vec::new(),
        });
        assert_eq!(usage_gross_subtotal(&[taxed], true).unwrap(), 1_199);
    }

    #[test]
    fn usage_credit_pays_a_usage_line_with_its_tax() {
        let mut usage = line(1_000, true, LineItemKind::Regular);
        usage.tax_rate = Decimal::new(2, 1);
        usage.tax_amount = 200;
        usage.amount_total = 1_200;
        let request = credit_request(&[usage], &LineOwners::default(), None, 1_200, 0).unwrap();
        assert_eq!(request.requested_minor, 1_200);
        assert_eq!(usage_of(&request), 1_200);
    }

    #[test]
    fn a_consolidated_invoice_asks_each_subscription_for_its_own_lines() {
        use common_domain::ids::SubscriptionPriceComponentId;
        let (a, b) = (SubscriptionId::new(), SubscriptionId::new());
        let mut a_usage = line(1_000, true, LineItemKind::Regular);
        a_usage.subscription_id = Some(a);
        let mut b_usage = line(2_000, true, LineItemKind::Regular);
        b_usage.subscription_id = Some(b);
        // Stored before lines recorded their subscription: its component names it.
        let mut b_fee = line(500, false, LineItemKind::Regular);
        b_fee.sub_component_id = Some(SubscriptionPriceComponentId::new());
        let owners = LineOwners::for_components([(b_fee.sub_component_id.unwrap(), b)]);
        let request = credit_request(&[a_usage, b_usage, b_fee], &owners, None, 3_500, 0).unwrap();
        let mut demands = request
            .demands
            .iter()
            .map(|demand| (demand.subscription_id, demand.usage, demand.amount_minor))
            .collect::<Vec<_>>();
        demands
            .sort_by_key(|(subscription, usage, _)| (subscription.map(|id| id.as_uuid()), *usage));
        let mut expected = vec![
            (Some(a), true, 1_000),
            (Some(b), true, 2_000),
            (Some(b), false, 500),
        ];
        expected
            .sort_by_key(|(subscription, usage, _)| (subscription.map(|id| id.as_uuid()), *usage));
        assert_eq!(demands, expected);
    }

    /// Late usage of a closed period stays its own demand, dated before that period's end, so
    /// only credit effective then pays it; a fee carries no date.
    #[test]
    fn usage_lines_ask_by_the_end_of_their_dates() {
        let september = line(1_000, true, LineItemKind::Regular);
        let mut october = line(2_000, true, LineItemKind::Regular);
        october.start_date = september.end_date;
        october.end_date = september.end_date + chrono::Days::new(31);
        let fee = line(500, false, LineItemKind::Regular);
        let request = credit_request(
            &[september.clone(), october.clone(), fee],
            &LineOwners::default(),
            None,
            3_500,
            0,
        )
        .unwrap();
        let dated = request
            .demands
            .iter()
            .map(|demand| (demand.usage, demand.amount_minor, demand.dated_before))
            .collect::<Vec<_>>();
        assert_eq!(
            dated,
            vec![
                (true, 1_000, Some(september.usage_dated_before())),
                (true, 2_000, Some(october.usage_dated_before())),
                (false, 500, None),
            ]
        );
    }

    #[test]
    fn paid_credit_is_asked_for_what_promotional_credit_left() {
        let mut usage = line(1_000, true, LineItemKind::Regular);
        usage.promotional_credit = Some(600);
        usage.taxable_amount = 400;
        usage.amount_total = 400;
        let request = credit_request(&[usage], &LineOwners::default(), None, 400, 0).unwrap();
        assert_eq!(request.requested_minor, 400);
        assert_eq!(usage_of(&request), 400);
    }
}
