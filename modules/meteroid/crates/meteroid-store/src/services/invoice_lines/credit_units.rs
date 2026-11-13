//! Usage priced in a credit unit is paid first by the customer's wallet in that unit; only what it
//! does not cover is billed in money, at the price's rate, before discounts and tax (Orb's
//! unit-first, remainder-to-fiat pipeline). Covered credits are never taxed.
use crate::StoreResult;
use crate::domain::invoice_lines::LineCreditUnit;
use crate::domain::{LineItem, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::credit::allocation::{
    CreditDemand, CreditRequest, GrantPool, allocate, assign,
};
use crate::repositories::credit::customer_credit::{self as credit, BilledPeriods};
use crate::services::Services;
use crate::services::credit::forecast::{CreditBooks, Drawn};
use crate::services::credit::unit_wallets::OverageRates;
use crate::services::credit::{billing_windows, expiry_settlement};
use crate::services::invoice_lines::component::PinnedPrices;
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{BaseId, CreditUnitId, InvoiceId, PlanVersionId, SubscriptionId};
use diesel_models::customer_credit::{CreditUsageSettlementRow, CustomerCreditAccountRow};
use error_stack::Report;

/// How the credit-priced lines of an invoice learn what their unit wallets pay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreditUnitCover {
    /// Drafts and previews: what the wallets would pay now, read without locks.
    Estimate { invoice_id: Option<InvoiceId> },
    /// Finalization: drawn under the unit's grant locks, recorded against `invoice_id`, from the
    /// credit as it stood at the issue moment `at`.
    Draw {
        invoice_id: InvoiceId,
        at: credit::MovementTime,
    },
    /// Finalization of lines that stand as computed: draws exactly what they show covered, and
    /// refuses when the wallets no longer hold it (a recompute then prices the change).
    DrawAsShown {
        invoice_id: InvoiceId,
        at: credit::MovementTime,
    },
}

/// The lines one subscription period contributes to an invoice, with what locating its usage
/// windows needs.
pub(crate) struct CoverTarget<'a> {
    pub details: &'a SubscriptionDetails,
    pub invoice_date: NaiveDate,
    pub cycle_index: u32,
    pub span: super::invoice_lines::BilledSpan,
    pub currency_precision: u8,
    pub lines: &'a mut Vec<LineItem>,
}

fn invalid(message: &str) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.to_owned()))
}

/// Money owed for the credits of `block` its wallet did not cover, in minor units of the invoice
/// currency, rounded once, halves away from zero.
pub(crate) fn overage_money(block: &LineCreditUnit, currency_precision: u8) -> StoreResult<i64> {
    let uncovered = block
        .consumed
        .checked_sub(block.covered)
        .filter(|uncovered| *uncovered >= 0)
        .ok_or_else(|| invalid("a line covers more credits than it consumed"))?;
    crate::domain::credit_units::beyond_balance_money_minor(
        uncovered,
        block.precision,
        block.rate,
        currency_precision,
    )
}

/// Splits `covered` over lines pro rata to what each consumed, largest remainder first, ties to
/// the earlier line: independent of tax categories and exact to the minor unit.
pub fn allocate_covered(consumed: &[i64], covered: i64) -> Vec<i64> {
    let total: i128 = consumed.iter().map(|c| i128::from(*c.max(&0))).sum();
    if total <= 0 || covered <= 0 {
        return vec![0; consumed.len()];
    }
    let covered = i128::from(covered).min(total);
    let mut shares: Vec<(i128, i128)> = consumed
        .iter()
        .map(|c| {
            let scaled = i128::from(*c.max(&0)) * covered;
            (scaled / total, scaled % total)
        })
        .collect();
    let mut left = covered - shares.iter().map(|(share, _)| share).sum::<i128>();
    let mut order: Vec<usize> = (0..shares.len()).collect();
    order.sort_by(|a, b| shares[*b].1.cmp(&shares[*a].1).then(a.cmp(b)));
    for index in order {
        if left <= 0 {
            break;
        }
        if shares[index].1 > 0 {
            shares[index].0 += 1;
            left -= 1;
        }
    }
    shares
        .into_iter()
        .map(|(share, _)| i64::try_from(share).unwrap_or(i64::MAX))
        .collect()
}

/// One subscription's usage priced in one unit, of lines ending at `dated_before`.
#[derive(Debug, Clone)]
struct SubscriptionUse {
    subscription_id: SubscriptionId,
    dated_before: DateTime<Utc>,
    consumed: i64,
    shown_covered: i64,
}

impl SubscriptionUse {
    fn holds(&self, subscription_id: SubscriptionId, line: &LineItem) -> bool {
        self.subscription_id == subscription_id && self.dated_before == line.usage_dated_before()
    }
}

/// The usage priced in one unit, by subscription and the end of its lines' dates: only a
/// subscription's own credit and customer-wide credit effective before that end pay its share,
/// and each share is covered (and repriced) on its own lines.
#[derive(Debug, Clone)]
struct UnitDemand {
    unit_id: CreditUnitId,
    code: String,
    uses: Vec<SubscriptionUse>,
}

impl UnitDemand {
    fn consumed(&self) -> StoreResult<i64> {
        self.uses
            .iter()
            .try_fold(0_i64, |sum, used| sum.checked_add(used.consumed))
            .ok_or_else(|| invalid("credit usage exceeds the supported range"))
    }

    /// One demand per subscription, in `uses` order, each at most `amount_of` its use.
    fn request(&self, amount_of: impl Fn(&SubscriptionUse) -> i64) -> CreditRequest {
        let demands = self
            .uses
            .iter()
            .map(|used| CreditDemand {
                subscription_id: Some(used.subscription_id),
                product_id: None,
                usage: true,
                amount_minor: amount_of(used).max(0),
                dated_before: Some(used.dated_before),
            })
            .collect::<Vec<_>>();
        CreditRequest {
            requested_minor: demands.iter().map(|demand| demand.amount_minor).sum(),
            demands,
        }
    }

    fn periods(&self, targets: &[CoverTarget<'_>]) -> Vec<(SubscriptionId, NaiveDate)> {
        let mut periods = unit_periods(targets, self.unit_id);
        periods.sort_by_key(|(subscription_id, start)| (subscription_id.as_uuid(), *start));
        periods.dedup();
        periods
    }
}

fn demands(targets: &[CoverTarget<'_>]) -> StoreResult<Vec<UnitDemand>> {
    let overflow = || invalid("credit usage exceeds the supported range");
    let mut demands: Vec<UnitDemand> = Vec::new();
    for (subscription_id, line, block) in targets.iter().flat_map(|target| {
        target.lines.iter().filter_map(|line| {
            line.credit_unit
                .as_ref()
                .map(|block| (target.details.subscription.id, line, block))
        })
    }) {
        let index = match demands.iter().position(|d| d.unit_id == block.unit_id) {
            Some(index) => index,
            None => {
                demands.push(UnitDemand {
                    unit_id: block.unit_id,
                    code: credit::normalized_currency(&block.code),
                    uses: Vec::new(),
                });
                demands.len() - 1
            }
        };
        let uses = &mut demands[index].uses;
        match uses
            .iter_mut()
            .find(|used| used.holds(subscription_id, line))
        {
            Some(used) => {
                used.consumed = used
                    .consumed
                    .checked_add(block.consumed)
                    .ok_or_else(overflow)?;
                used.shown_covered = used
                    .shown_covered
                    .checked_add(block.covered)
                    .ok_or_else(overflow)?;
            }
            None => uses.push(SubscriptionUse {
                subscription_id,
                dated_before: line.usage_dated_before(),
                consumed: block.consumed,
                shown_covered: block.covered,
            }),
        }
    }
    for demand in &mut demands {
        demand
            .uses
            .sort_by_key(|used| (used.subscription_id.as_uuid(), used.dated_before));
    }
    Ok(demands)
}

/// The `(subscription, period start)` pairs whose settlements of this unit the invoice takes over.
fn unit_periods(
    targets: &[CoverTarget<'_>],
    unit_id: CreditUnitId,
) -> Vec<(SubscriptionId, NaiveDate)> {
    targets
        .iter()
        .flat_map(|target| {
            let subscription_id = target.details.subscription.id;
            target
                .lines
                .iter()
                .filter(move |line| {
                    line.credit_unit
                        .as_ref()
                        .is_some_and(|block| block.unit_id == unit_id)
                })
                .map(move |line| (subscription_id, line.start_date))
        })
        .collect()
}

impl Services {
    /// Sets what each credit-priced line's wallet covers and reprices its money fields to the
    /// remainder. A no-op for an invoice with no credit-priced line.
    pub(crate) async fn cover_credit_unit_lines(
        &self,
        conn: &mut PgConn,
        cover: CreditUnitCover,
        targets: &mut [CoverTarget<'_>],
    ) -> StoreResult<()> {
        self.cover_credit_unit_lines_on(conn, cover, targets, None)
            .await
    }

    /// `cover_credit_unit_lines`, estimating on a forecast's `book` when given.
    pub(crate) async fn cover_credit_unit_lines_on(
        &self,
        conn: &mut PgConn,
        cover: CreditUnitCover,
        targets: &mut [CoverTarget<'_>],
        book: Option<&CreditBooks>,
    ) -> StoreResult<()> {
        let demands = demands(targets)?;
        let Some(first) = targets.first() else {
            return Ok(());
        };
        if demands.is_empty() {
            return Ok(());
        }
        let tenant_id = first.details.subscription.tenant_id;
        let customer_id = first.details.subscription.customer_id;
        if targets.iter().any(|target| {
            target.details.subscription.tenant_id != tenant_id
                || target.details.subscription.customer_id != customer_id
        }) {
            return Err(invalid("an invoice bills a single customer"));
        }
        let now = common_domain::clock::now();
        let unit_ids: Vec<CreditUnitId> = demands.iter().map(|demand| demand.unit_id).collect();
        let plan_version_ids: Vec<PlanVersionId> = targets
            .iter()
            .map(|target| target.details.subscription.plan_version_id)
            .collect();
        let mut rates = OverageRates::load(
            conn,
            tenant_id,
            customer_id,
            &first.details.subscription.currency,
            &unit_ids,
            &plan_version_ids,
        )
        .await?;
        for demand in demands {
            if demand.consumed()? <= 0 {
                continue;
            }
            let covered = match cover {
                CreditUnitCover::Estimate { invoice_id } => {
                    self.estimate_unit_cover(conn, targets, &demand, invoice_id, now, book)
                        .await?
                }
                CreditUnitCover::Draw { invoice_id, at } => {
                    let request = demand.request(|used| used.consumed);
                    self.draw_unit_cover(conn, targets, &demand, &request, invoice_id, at)
                        .await?
                }
                CreditUnitCover::DrawAsShown { invoice_id, at } => {
                    let request = demand.request(|used| used.shown_covered);
                    let drawn = self
                        .draw_unit_cover(conn, targets, &demand, &request, invoice_id, at)
                        .await?;
                    let drawn: i64 = drawn.into_iter().sum();
                    let shown: i64 = demand.uses.iter().map(|used| used.shown_covered).sum();
                    if drawn != shown {
                        return Err(Report::new(StoreError::InvoiceComputationError).attach(
                            format!(
                                "the {} balance no longer covers what the invoice shows \
                                 ({drawn} of {shown}); recompute it",
                                demand.code
                            ),
                        ));
                    }
                    continue;
                }
            };
            for (used, covered) in demand.uses.iter().zip(covered) {
                if covered > used.consumed {
                    return Err(invalid(
                        "the credit an invoice holds exceeds the usage it prices in that unit",
                    ));
                }
                let of_subscription = |target: &CoverTarget<'_>| {
                    target.details.subscription.id == used.subscription_id
                };
                let consumed = targets
                    .iter()
                    .filter(|target| of_subscription(target))
                    .flat_map(|target| target.lines.iter())
                    .filter(|line| used.holds(used.subscription_id, line))
                    .filter_map(|line| line.credit_unit.as_ref())
                    .filter(|block| block.unit_id == demand.unit_id)
                    .map(|block| block.consumed)
                    .collect::<Vec<_>>();
                let mut shares = allocate_covered(&consumed, covered).into_iter();
                for target in targets.iter_mut().filter(|target| of_subscription(target)) {
                    let precision = target.currency_precision;
                    let plan_version_id = target.details.subscription.plan_version_id;
                    for line in target.lines.iter_mut() {
                        if !used.holds(used.subscription_id, line) {
                            continue;
                        }
                        let Some(block) = line
                            .credit_unit
                            .as_mut()
                            .filter(|block| block.unit_id == demand.unit_id)
                        else {
                            continue;
                        };
                        block.covered = shares.next().unwrap_or(0);
                        let price_rate = *block.price_rate.get_or_insert(block.rate);
                        block.rate =
                            rates.rate(block.unit_id, &block.code, plan_version_id, price_rate);
                        let money = overage_money(block, precision)?;
                        line.amount_subtotal = money;
                        line.taxable_amount = money;
                        line.amount_total = money;
                        line.tax_amount = 0;
                    }
                }
            }
        }
        Ok(())
    }

    /// What each subscription of `demand` would have covered: the open settlements of its periods
    /// the invoice would keep, then a draw on the rest.
    async fn estimate_unit_cover(
        &self,
        conn: &mut PgConn,
        targets: &[CoverTarget<'_>],
        demand: &UnitDemand,
        invoice_id: Option<InvoiceId>,
        now: DateTime<Utc>,
        book: Option<&CreditBooks>,
    ) -> StoreResult<Vec<i64>> {
        let tenant_id = targets[0].details.subscription.tenant_id;
        let customer_id = targets[0].details.subscription.customer_id;
        let accounts = CustomerCreditAccountRow::list_for_customer(conn, tenant_id, customer_id)
            .await?
            .into_iter()
            .filter(|account| account.currency == demand.code)
            .map(|account| account.id)
            .collect::<Vec<_>>();
        let mut kept_settlements = Vec::new();
        for (subscription_id, period_start) in demand.periods(targets) {
            for settlement in CreditUsageSettlementRow::list_for_period(
                conn,
                tenant_id,
                subscription_id,
                period_start,
                &demand.code,
            )
            .await?
            {
                let ours = settlement.invoice_id.is_none() || settlement.invoice_id == invoice_id;
                if ours && accounts.contains(&settlement.account_id) {
                    kept_settlements.push(settlement.id.as_uuid());
                }
            }
        }
        // Placed on the uses as finalizing places them (`net_settlements_then_draw_tx`): by what
        // each settling grant may pay, whichever line of the period it settled.
        let consumed = demand.request(|used| used.consumed);
        let held = credit::settlement_holders_tx(conn, tenant_id, &kept_settlements).await?;
        let kept = assign(&held, &consumed)?;
        let request = consumed.remainder(&kept)?;
        let fresh = match book {
            Some(book) => {
                let candidates = book.candidates_tx(conn, &demand.code).await?;
                let fresh = allocate(&candidates, &request, now, GrantPool::All)?;
                book.record(Drawn {
                    denomination: demand.code.clone(),
                    pool: GrantPool::All,
                    held: kept.clone().then(fresh.clone())?,
                    fresh: fresh.clone(),
                })?;
                fresh
            }
            None => {
                credit::preview_estimate_tx(
                    conn,
                    tenant_id,
                    customer_id,
                    &demand.code,
                    GrantPool::All,
                    &request,
                    now,
                )
                .await?
            }
        };
        Ok(demand
            .uses
            .iter()
            .enumerate()
            .map(|(index, used)| {
                kept.covered
                    .get(index)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(fresh.covered.get(index).copied().unwrap_or(0))
                    .min(used.consumed)
            })
            .collect())
    }

    /// Draws `request` (one demand per subscription of `demand`), returning what each covers.
    async fn draw_unit_cover(
        &self,
        conn: &mut PgConn,
        targets: &[CoverTarget<'_>],
        demand: &UnitDemand,
        request: &CreditRequest,
        invoice_id: InvoiceId,
        time: credit::MovementTime,
    ) -> StoreResult<Vec<i64>> {
        let tenant_id = targets[0].details.subscription.tenant_id;
        let customer_id = targets[0].details.subscription.customer_id;
        let lock = credit::lock_customer_tx(conn, tenant_id, customer_id).await?;
        let mut grants = credit::lock_grants_tx(conn, &lock, &demand.code).await?;
        if grants.is_empty() {
            return Ok(vec![0; demand.uses.len()]);
        }
        let mut windows = Vec::new();
        let mut periods = demand.periods(targets);
        let mut pinned = PinnedPrices::default();
        for target in targets {
            let unit_windows = billing_windows::windows_at(
                conn,
                target.details,
                target.invoice_date,
                target.cycle_index,
                target.span,
            )
            .await?
            .into_iter()
            .filter(|window| window.currency == demand.code)
            .collect::<Vec<_>>();
            periods.extend(
                unit_windows
                    .iter()
                    .map(|window| (window.subscription_id, window.period.start)),
            );
            windows.extend(unit_windows);
            pinned.add_lines(target.lines.as_slice(), false);
        }
        periods.sort_by_key(|(subscription_id, start)| (subscription_id.as_uuid(), *start));
        periods.dedup();
        expiry_settlement::settle_due_expiries_tx(
            &self.store,
            self.usage_client.as_ref(),
            conn,
            tenant_id,
            customer_id,
            &mut grants,
            &windows,
            &pinned,
            &demand.code,
            expiry_settlement::UnitCover::AtBound,
            time,
        )
        .await?;
        let allocation = credit::net_settlements_then_draw_tx(
            conn,
            &*self.store.hooks.credit,
            &lock,
            invoice_id,
            &demand.code,
            &grants,
            &BilledPeriods { periods },
            GrantPool::All,
            request,
            &[],
            time,
        )
        .await?;
        Ok((0..demand.uses.len())
            .map(|index| allocation.covered.get(index).copied().unwrap_or(0))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    fn block(consumed: i64, covered: i64, precision: u8, rate: Decimal) -> LineCreditUnit {
        LineCreditUnit {
            unit_id: CreditUnitId::new(),
            code: "CREDITS".to_owned(),
            precision,
            rate,
            price_rate: None,
            unit_price: None,
            sub_lines: Vec::new(),
            consumed,
            covered,
        }
    }

    #[test]
    fn covered_credits_split_pro_rata_by_consumption() {
        assert_eq!(allocate_covered(&[300, 100], 200), vec![150, 50]);
        assert_eq!(allocate_covered(&[300, 100], 400), vec![300, 100]);
        assert_eq!(allocate_covered(&[300, 100], 0), vec![0, 0]);
    }

    #[test]
    fn the_remainder_goes_to_the_largest_fraction_then_the_earlier_line() {
        // Two units over three equal lines: the first two get one each.
        assert_eq!(allocate_covered(&[1, 1, 1], 2), vec![1, 1, 0]);
        assert_eq!(allocate_covered(&[10, 10, 10], 10), vec![4, 3, 3]);
        assert_eq!(allocate_covered(&[1, 2], 2), vec![1, 1]);
        let split = allocate_covered(&[7, 13, 29, 51], 37);
        assert_eq!(split.iter().sum::<i64>(), 37);
        assert!(split.iter().zip([7, 13, 29, 51]).all(|(s, c)| *s <= c));
    }

    #[test]
    fn coverage_never_exceeds_consumption() {
        assert_eq!(allocate_covered(&[5, 5], 1_000), vec![5, 5]);
        assert_eq!(allocate_covered(&[0, 0], 10), vec![0, 0]);
    }

    #[test]
    fn the_uncovered_credits_are_billed_in_money_at_the_rate() {
        // 3,600 credits (precision 2: 360000 minor), 3,000 covered, $0.01 per credit, USD.
        assert_eq!(
            overage_money(&block(360_000, 300_000, 2, dec!(0.01)), 2).unwrap(),
            600
        );
        // Sub-cent credits: 1,234 credits at precision 0, $0.001 each, rounded half away.
        assert_eq!(
            overage_money(&block(1_234, 0, 0, dec!(0.001)), 2).unwrap(),
            123
        );
        assert_eq!(
            overage_money(&block(1_235, 0, 0, dec!(0.001)), 2).unwrap(),
            124
        );
        // Finer unit than the currency (JPY, precision 0) and a fully covered line.
        assert_eq!(overage_money(&block(1_500, 0, 3, dec!(2)), 0).unwrap(), 3);
        assert_eq!(
            overage_money(&block(500, 500, 2, dec!(0.01)), 2).unwrap(),
            0
        );
        assert!(overage_money(&block(5, 6, 2, dec!(0.01)), 2).is_err());
    }
}
