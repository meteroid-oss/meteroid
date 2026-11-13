//! What a subscription's fees should have billed, as terms over their own periods (R-079), and
//! what an ending or an amendment takes back of their issued lines (R-080).

use std::collections::{BTreeSet, HashMap};

use billing_core::{Adjustment, Fee, IssuedLine, TakeBack, Term};
use chrono::NaiveDate;
use common_domain::ids::{BaseId, PriceComponentId, SubscriptionId, TenantId};
use diesel_models::enums::SlotTransactionStatusEnum;
use diesel_models::slot_transactions::SlotTransactionRow;
use rust_decimal::Decimal;

use crate::StoreResult;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::subscription_changes::{AddedComponent, RemovedComponent};
use crate::domain::subscription_components::SubscriptionFee;
use crate::domain::{Subscription, SubscriptionAddOn, SubscriptionComponent, SubscriptionDetails};
use crate::repositories::credit_notes::CreditLineItem;
use crate::services::subscriptions::amendment::PeriodCreditContext;
use crate::services::subscriptions::proration::{component_advance_amount_cents, own_period};
use crate::store::PgConn;

/// The seats a slot fee bills from each date its count changed, as its active transactions
/// record them, and the count before the first (a subscription older than its seed has one).
fn seat_steps(
    transactions: &[SlotTransactionRow],
    unit: &str,
) -> (Option<u32>, Vec<(NaiveDate, u32)>) {
    let mut active: Vec<&SlotTransactionRow> = transactions
        .iter()
        .filter(|t| t.unit == unit && t.status == SlotTransactionStatusEnum::Active)
        .collect();
    active.sort_by_key(|t| t.effective_at);
    let seats = |count: i32| u32::try_from(count).unwrap_or(0);
    (
        active.first().map(|t| seats(t.prev_active_slots)),
        active
            .iter()
            .map(|t| (t.effective_at.date(), seats(t.prev_active_slots + t.delta)))
            .collect(),
    )
}

/// The terms of `current` over `[from, until)`: one per seat count a slot fee billed.
fn priced_over(
    fee: &Fee,
    current: &SubscriptionFee,
    billing: &SubscriptionFeeBillingPeriod,
    period: (NaiveDate, NaiveDate),
    (from, until): (NaiveDate, NaiveDate),
    slots: &[SlotTransactionRow],
    precision: u8,
) -> Vec<Term> {
    let full =
        |priced: &SubscriptionFee| component_advance_amount_cents(priced, billing, precision);
    let SubscriptionFee::Slot { unit, .. } = current else {
        return vec![Term {
            fee: fee.clone(),
            full: full(current),
            period,
            from,
            until,
        }];
    };
    let (before, steps) = seat_steps(slots, unit);
    let seats_at = |at: NaiveDate| {
        steps
            .iter()
            .rev()
            .find(|(changed, _)| *changed <= at)
            .map(|(_, seats)| *seats)
            .or(before)
    };
    let mut cuts: Vec<NaiveDate> = std::iter::once(from)
        .chain(
            steps
                .iter()
                .map(|(at, _)| *at)
                .filter(|at| from < *at && *at < until),
        )
        .chain(std::iter::once(until))
        .collect();
    cuts.dedup();
    cuts.windows(2)
        .map(|span| {
            let mut priced = current.clone();
            if let (SubscriptionFee::Slot { initial_slots, .. }, Some(seats)) =
                (&mut priced, seats_at(span[0]))
            {
                *initial_slots = seats;
            }
            Term {
                fee: fee.clone(),
                full: full(&priced),
                period,
                from: span[0],
                until: span[1],
            }
        })
        .collect()
}

/// A row of a component or add-on: the price its fee billed from `from` until `to`.
pub(in crate::services) struct FeeRow {
    pub id: uuid::Uuid,
    pub fee: Fee,
    pub lineage: String,
    pub priced: SubscriptionFee,
    pub billing: SubscriptionFeeBillingPeriod,
    pub from: NaiveDate,
    pub to: Option<NaiveDate>,
}

impl FeeRow {
    pub(in crate::services) fn of_component(component: SubscriptionComponent) -> Self {
        let lineage = component.lineage();
        FeeRow {
            id: component.id.as_uuid(),
            fee: Fee::Component(lineage.as_uuid()),
            lineage: lineage.to_string(),
            priced: component.fee,
            billing: component.period,
            from: component.effective_from,
            to: component.effective_to,
        }
    }

    pub(in crate::services) fn of_add_on(add_on: &SubscriptionAddOn) -> Self {
        let lineage = add_on.lineage();
        FeeRow {
            id: add_on.id.as_uuid(),
            fee: Fee::AddOn(lineage.as_uuid()),
            lineage: lineage.to_string(),
            priced: super::terminate::removed_add_on(add_on).current_fee,
            billing: add_on.period,
            from: add_on.effective_from,
            to: add_on.effective_to,
        }
    }
}

/// A subscription's fee rows around a date, with what prices them: its seats and its grid.
pub(in crate::services) struct FeeHistory {
    pub subscription: SubscriptionId,
    pub rows: Vec<FeeRow>,
    pub slots: Vec<SlotTransactionRow>,
    pub precision: u8,
    /// The subscription's period containing the date, and the grid its cadences bill on.
    pub current: (NaiveDate, NaiveDate),
    pub cadence: BillingPeriodEnum,
    pub billing_start: NaiveDate,
    pub billing_day: u32,
}

impl FeeHistory {
    /// The rows of the year up to `at`.
    pub(in crate::services) async fn read(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        at: NaiveDate,
    ) -> StoreResult<Self> {
        use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
        use diesel_models::subscription_components::SubscriptionComponentRow;
        let subscription: Subscription =
            diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
                conn,
                &tenant_id,
                subscription_id,
            )
            .await?
            .try_into()?;
        let billing_start = subscription
            .billing_start_date
            .unwrap_or(subscription.current_period_start);
        let billing_day = u32::from(subscription.billing_day_anchor);
        let current = match subscription.current_period_end {
            Some(end) if subscription.current_period_start <= at && at < end => {
                (subscription.current_period_start, end)
            }
            _ => {
                let period = crate::utils::periods::find_period_containing_date(
                    billing_start,
                    at,
                    &subscription.period,
                    billing_day,
                );
                (period.start, period.end)
            }
        };
        let reach = at
            .checked_sub_months(chrono::Months::new(12))
            .unwrap_or(current.0)
            .min(current.0);
        // Rows starting on `at` included.
        let through = at.succ_opt().unwrap_or(at);
        let mut rows = Vec::new();
        for row in SubscriptionComponentRow::list_component_history_for_period(
            conn,
            &subscription_id,
            reach,
            through,
        )
        .await?
        {
            rows.push(FeeRow::of_component(row.try_into()?));
        }
        for row in SubscriptionAddOnRow::list_add_on_history_for_period(
            conn,
            &subscription_id,
            reach,
            through,
        )
        .await?
        {
            let add_on: SubscriptionAddOn = row.try_into()?;
            rows.push(FeeRow::of_add_on(&add_on));
        }
        let slots = SlotTransactionRow::list_by_subscription_id(
            conn,
            tenant_id,
            subscription_id,
            None,
            None,
        )
        .await?;
        Ok(FeeHistory {
            subscription: subscription_id,
            rows,
            slots,
            precision: crate::constants::Currencies::resolve_currency_precision(
                &subscription.currency,
            )
            .unwrap_or(2),
            current,
            cadence: subscription.period,
            billing_start,
            billing_day,
        })
    }

    /// The terms of the fees of `lineages` up to `at`, each row over its own period: the grid's,
    /// or, off it (a cadence changed), the window of the line billing the fee across `at`.
    fn terms_until(
        &self,
        at: NaiveDate,
        lineages: &BTreeSet<String>,
        issued: &[IssuedLine],
    ) -> Vec<Term> {
        let grid = own_period(
            self.current,
            self.cadence,
            self.billing_start,
            self.billing_day,
            at,
        );
        let mut terms = Vec::new();
        for row in self.rows.iter().filter(|row| {
            lineages.contains(&row.lineage)
                && !matches!(row.billing, SubscriptionFeeBillingPeriod::OneTime)
        }) {
            let on_grid = grid(&row.billing);
            let until = row.to.map_or(at, |to| to.min(at));
            let period = issued
                .iter()
                .filter(|line| {
                    line.key.fee == row.fee
                        && line.from <= at
                        && at < line.until
                        && row.from < line.until
                        && line.from < row.to.unwrap_or(line.until)
                        && !(on_grid.0 <= line.from && line.until <= on_grid.1)
                })
                .max_by(|a, b| a.from.cmp(&b.from).then_with(|| a.id.cmp(&b.id)))
                .map_or(on_grid, |line| (line.from, line.until));
            let from = row.from.max(period.0);
            let until = until.max(from);
            if from == until {
                terms.push(Term {
                    fee: row.fee.clone(),
                    full: 0,
                    period,
                    from,
                    until,
                });
                continue;
            }
            terms.extend(priced_over(
                &row.fee,
                &row.priced,
                &row.billing,
                period,
                (from, until),
                &self.slots,
                self.precision,
            ));
        }
        terms
    }

    /// The fee of each of `lineages`, by lineage.
    fn fees(&self, lineages: &BTreeSet<String>) -> HashMap<&str, &Fee> {
        self.rows
            .iter()
            .filter(|row| lineages.contains(&row.lineage))
            .map(|row| (row.lineage.as_str(), &row.fee))
            .collect()
    }
}

/// What the fees of `lineages` ending at `at` take back of the lines billing them from it or
/// before: what reconcile owes back of what they billed over what their terms bill up to `at`,
/// each line by its unused time (R-077, R-080). A seat line, which names only its plan component, counts
/// under the fee of the lineage of that component.
pub(in crate::services) fn ending_credits(
    history: &FeeHistory,
    at: NaiveDate,
    lineages: &BTreeSet<String>,
    plan_components: &HashMap<String, PriceComponentId>,
    ctx: &PeriodCreditContext,
) -> Vec<CreditLineItem> {
    let fees = history.fees(lineages);
    let subscription = Some(history.subscription.as_uuid());
    // A row continuing its lineage from the ending (a carried fee's new price) is not what it
    // ends.
    let continued: Vec<uuid::Uuid> = history
        .rows
        .iter()
        .filter(|row| {
            row.from >= at
                && lineages.contains(&row.lineage)
                && history
                    .rows
                    .iter()
                    .any(|ended| ended.lineage == row.lineage && ended.to == Some(row.from))
        })
        .map(|row| row.id)
        .collect();
    let mut issued = ctx.issued_lines(|_| None);
    issued.retain(|line| {
        line.from <= at
            && !ctx
                .all_lines
                .iter()
                .find(|l| l.local_id == line.id)
                .and_then(|l| {
                    l.sub_component_id
                        .map(|id| id.as_uuid())
                        .or(l.sub_add_on_id.map(|id| id.as_uuid()))
                })
                .is_some_and(|row| continued.contains(&row))
    });
    for line in &mut issued {
        line.key.subscription = line.key.subscription.or(subscription);
        if line.key.fee != Fee::Unattached {
            continue;
        }
        let price_component = ctx
            .all_lines
            .iter()
            .find(|l| l.local_id == line.id)
            .and_then(|l| l.price_component_id);
        if let Some(fee) = plan_components
            .iter()
            .filter(|(_, plan)| Some(**plan) == price_component)
            .filter_map(|(lineage, _)| Some((lineage, fees.get(lineage.as_str())?)))
            .min_by_key(|(lineage, _)| lineage.as_str())
            .map(|(_, fee)| fee)
        {
            line.key.fee = (*fee).clone();
        }
    }
    let terms = history.terms_until(at, lineages, &issued);
    let expected = billing_core::expected(subscription, &terms);
    let mut credits = Vec::new();
    for adjustment in billing_core::reconcile(&expected, &issued, TakeBack::Unused(at)) {
        match adjustment {
            Adjustment::Credit { line, amount } => {
                if let Some(line) = issued.iter().find(|issued| issued.id == line) {
                    credits.push(CreditLineItem::Amount {
                        local_id: line.id.clone(),
                        amount_cents: line.net_of(amount),
                    });
                }
            }
            // Billed short up to `at` (a draft not issued yet): its invoice bills it.
            Adjustment::Bill { .. } => {}
            Adjustment::Short { key, amount } => log::warn!(
                "the ending on {at} of subscription {} owes back {amount} of {:?} beyond what its lines have left",
                history.subscription,
                key.fee
            ),
        }
    }
    credits
}

/// The fee a lineage names, among the subscription's components and add-ons.
fn fee_of(details: &SubscriptionDetails, lineage: &str) -> Option<Fee> {
    details
        .price_components
        .iter()
        .find(|c| c.lineage().to_string() == lineage)
        .map(|c| Fee::Component(c.lineage().as_uuid()))
        .or_else(|| {
            details
                .add_ons
                .iter()
                .find(|a| a.lineage().to_string() == lineage)
                .map(|a| Fee::AddOn(a.lineage().as_uuid()))
        })
}

/// What a credit of `net` of a line reads as on a note: a share of its quantity when it was
/// billed by the unit, else an amount.
fn taken_back(line: &IssuedLine, net: i64) -> CreditLineItem {
    match line.quantity {
        Some(quantity) if line.unit_priced && quantity > Decimal::ZERO && line.subtotal > 0 => {
            CreditLineItem::Line {
                local_id: line.id.clone(),
                quantity: (Decimal::from(net) / Decimal::from(line.subtotal) * quantity)
                    .min(quantity),
            }
        }
        _ => CreditLineItem::Amount {
            local_id: line.id.clone(),
            amount_cents: net,
        },
    }
}

/// An immediate amendment at `change`: the fees it ends and starts, each on its own period.
pub(in crate::services) struct AmendmentChange<'a> {
    pub details: &'a SubscriptionDetails,
    pub added: &'a [AddedComponent],
    pub removed: &'a [RemovedComponent],
    pub change: NaiveDate,
    pub own_period: &'a (dyn Fn(&SubscriptionFeeBillingPeriod) -> (NaiveDate, NaiveDate) + Sync),
    pub precision: u8,
}

impl AmendmentChange<'_> {
    /// Each touched fee's terms: what its rows billed up to the change, overrides earlier in
    /// the period included, then the price that starts there, to the end of its own period.
    fn terms(&self, history: &FeeHistory, issued: &[IssuedLine]) -> Vec<Term> {
        let lineages: BTreeSet<String> = self
            .removed
            .iter()
            .filter_map(|removed| removed.net_key.clone())
            .collect();
        let mut terms = history.terms_until(self.change, &lineages, issued);
        for added in self.added {
            if matches!(added.fee, SubscriptionFee::OneTime { .. }) {
                continue;
            }
            let fee = match (
                &added.net_key,
                added.billed_component_id,
                added.billed_add_on_id,
            ) {
                (Some(key), _, _) => fee_of(self.details, key),
                (None, Some(id), _) => Some(Fee::Component(id.as_uuid())),
                (None, None, Some(id)) => Some(Fee::AddOn(id.as_uuid())),
                (None, None, None) => None,
            };
            let Some(fee) = fee else { continue };
            let period = (self.own_period)(&added.period);
            terms.push(Term {
                fee,
                full: component_advance_amount_cents(&added.fee, &added.period, self.precision),
                period,
                from: self.change,
                until: period.1,
            });
        }
        terms
    }

    /// The period's issued lines, a seat purchase (which names only the plan's component) under
    /// the fee of the subscription's component of that price.
    fn issued(&self, ctx: &PeriodCreditContext) -> Vec<IssuedLine> {
        let mut issued = ctx.issued_lines(|_| None);
        let subscription = Some(self.details.subscription.id.as_uuid());
        for line in &mut issued {
            line.key.subscription = line.key.subscription.or(subscription);
            if line.key.fee != Fee::Unattached {
                continue;
            }
            let price_component = ctx
                .all_lines
                .iter()
                .find(|l| l.local_id == line.id)
                .and_then(|l| l.price_component_id);
            if let Some(component) = self
                .details
                .price_components
                .iter()
                .find(|c| price_component.is_some() && c.price_component_id == price_component)
            {
                line.key.fee = Fee::Component(component.lineage().as_uuid());
            }
        }
        issued
    }

    /// What the amendment takes back of the period's issued lines, before its adjustment invoice
    /// bills what it adds: per fee, what they billed over what its terms bill, the latest lines
    /// first (R-078, R-080).
    pub(in crate::services) async fn credits(
        &self,
        conn: &mut PgConn,
        ctx: &PeriodCreditContext,
    ) -> StoreResult<Vec<CreditLineItem>> {
        let subscription = &self.details.subscription;
        let history =
            FeeHistory::read(conn, subscription.tenant_id, subscription.id, self.change).await?;
        let issued = self.issued(ctx);
        let expected = billing_core::expected(
            Some(subscription.id.as_uuid()),
            &self.terms(&history, &issued),
        );
        let mut credits = Vec::new();
        for adjustment in billing_core::reconcile(&expected, &issued, TakeBack::LatestFirst) {
            match adjustment {
                Adjustment::Credit { line, amount } => {
                    if let Some(line) = issued.iter().find(|issued| issued.id == line) {
                        credits.push(taken_back(line, line.net_of(amount)));
                    }
                }
                // Billed by the adjustment invoice.
                Adjustment::Bill { .. } => {}
                Adjustment::Short { key, amount } => log::warn!(
                    "the amendment on {} of subscription {} owes back {amount} of {:?} beyond what its lines have left",
                    self.change,
                    subscription.id,
                    key.fee
                ),
            }
        }
        Ok(credits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LineItemKind;
    use crate::domain::invoice_lines::LineItem;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn line(local_id: &str, subtotal: i64, start: NaiveDate, plan: PriceComponentId) -> LineItem {
        LineItem {
            local_id: local_id.to_owned(),
            name: "Seats".to_owned(),
            charge_type: None,
            amount_subtotal: subtotal,
            tax_rate: Decimal::ZERO,
            taxable_amount: subtotal,
            tax_amount: 0,
            amount_total: subtotal,
            tax_details: vec![],
            quantity: None,
            unit_price: None,
            start_date: start,
            end_date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: Some(plan),
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

    /// A 310.00 monthly fee billed on October 1, then raised to 620.00 on October 11 and billed
    /// 420.00 more for its 21 days: ending on October 17, each line gives back its 15 days left.
    /// The lines name only the plan's component, and no subscription.
    #[test]
    fn an_ending_takes_back_each_line_of_the_fee_by_its_unused_time() {
        let plan = PriceComponentId::new();
        let fee = Fee::Component(uuid::Uuid::now_v7());
        let row = |rate: i64, from: u32, to: Option<u32>| FeeRow {
            id: uuid::Uuid::now_v7(),
            fee: fee.clone(),
            lineage: "platform".to_owned(),
            priced: SubscriptionFee::Rate {
                rate: Decimal::from(rate),
            },
            billing: SubscriptionFeeBillingPeriod::Monthly,
            from: day(from),
            to: to.map(day),
        };
        let history = FeeHistory {
            subscription: SubscriptionId::new(),
            rows: vec![row(310, 1, Some(11)), row(620, 11, None)],
            slots: vec![],
            precision: 2,
            current: (day(1), NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()),
            cadence: BillingPeriodEnum::Monthly,
            billing_start: day(1),
            billing_day: 1,
        };
        let ctx = PeriodCreditContext {
            invoices: vec![],
            all_lines: vec![
                line("first", 31_000, day(1), plan),
                line("raised", 21_000, day(11), plan),
            ],
            line_to_invoice: Default::default(),
            line_lineage: Default::default(),
            already_credited: Default::default(),
            corrected: Default::default(),
            returned: Default::default(),
        };
        let mut credits: Vec<(String, i64)> = ending_credits(
            &history,
            day(17),
            &["platform".to_owned()].into_iter().collect(),
            &[("platform".to_owned(), plan)].into_iter().collect(),
            &ctx,
        )
        .into_iter()
        .map(|item| match item {
            CreditLineItem::Amount {
                local_id,
                amount_cents,
            } => (local_id, amount_cents),
            _ => unreachable!(),
        })
        .collect();
        credits.sort();
        assert_eq!(
            credits,
            vec![("first".to_owned(), 15_000), ("raised".to_owned(), 15_000)]
        );
    }
}
