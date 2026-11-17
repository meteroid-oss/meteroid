//! Minimum commitment as a period true-up: what of it to load, and its lines.
//!
//! The minimum of a completed period P is trued up on the Recurring invoice that closes P: a
//! renewal bills the period just ended, a termination the part of it served. P's in-scope spend
//! is its arrears on that invoice plus what the subscription's other invoices dated in P billed
//! for P (the advance fees of its opening invoice, immediate increases). What it counts and owes
//! is `billing_core::minimum`'s; this module loads its inputs and renders its lines. A
//! product-scoped minimum bills one line per scoped product, each taxed with its product (OQ17).

use std::collections::HashMap;

use chrono::NaiveDate;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};

use crate::StoreResult;
use crate::domain::enums::{BillingPeriodEnum, InvoiceType};
use crate::domain::plan_minimums::MinimumScope;
use crate::domain::subscription_minimums::SubscriptionMinimum;
use crate::domain::{
    AppliedCouponDetailed, ChargeType, LineItem, LineItemKind, SubscriptionDetails,
};
use crate::errors::StoreError;
use crate::services::invoice_lines::charges::{Lineages, evaluated_charge_key};
use crate::services::invoice_lines::discount::Queue;
use crate::store::PgConn;
use billing_core::charge::{ChargeKind, Fee};
use billing_core::minimum::{self as core, Spend};
use billing_core::proration::DaySpan;
use common_domain::ids::{AppliedCouponId, InvoiceId, ProductId, SubscriptionId, TenantId};
use error_stack::Report;

/// Display name shown on the invoice for the synthesized shortfall line.
const MINIMUM_COMMITMENT_LINE_NAME: &str = "Minimum commitment";

/// What a Switch event applied at a period's end records of the terms the period ran under.
pub(crate) const OUTGOING_MINIMUM_KEY: &str = "outgoing_minimum";
pub(crate) const OUTGOING_PERIOD_KEY: &str = "outgoing_period";

/// The true-up of the period an invoice closes, with what was billed for that period elsewhere.
#[derive(Clone, Debug, PartialEq)]
pub struct PeriodTrueUp {
    pub start: NaiveDate,
    /// Exclusive: the invoice date, the period's end or the day it was cut short.
    pub end: NaiveDate,
    pub minimum: SubscriptionMinimum,
    /// The period, its floor and what it nets, as the core trues it up.
    pub true_up: core::TrueUp,
    /// The lines of the subscription's other invoices billing the period.
    pub prior: Vec<Spend>,
    /// How that rule dated its shortfalls on the subscription, read only when the period or the
    /// invoice closing it carries one.
    pub up_front: Option<UpFrontShortfall>,
    /// The drafts among them: each is issued with the coupons it is owed then, not those it
    /// shows (R-022, R-089).
    pub provisional: Vec<Provisional>,
    /// The subscription's open drafts, the invoice closing the period in its place.
    pub queue: Queue,
}

/// A draft's lines in `prior`, net of the coupons it shows; a coupon's limited uses go to the
/// oldest periods first (R-089), so it is issued with those its place among the open drafts
/// owes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Provisional {
    pub id: InvoiceId,
    /// The coupons it shows.
    pub shown: Vec<AppliedCouponId>,
    /// Its lines' quoted subtotals before coupons, but a true-up's, which no coupon discounts.
    pub quoted: Vec<i64>,
    /// Its lines in `prior`: each one's place there and in `quoted`.
    pub lines: Vec<(usize, usize)>,
}

/// How the rule before the true-up dated the shortfalls it charged on a subscription's invoices:
/// one line per invoice its floor applied on, dated over the period that invoice opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpFrontShortfall {
    /// The floor applied on the opening invoice: each line prepaid the period it is dated over.
    Prepaid,
    /// The floor waited for the first renewal: each line trued up the period before its own.
    TrueUpOfPeriodBefore,
}

/// The period an invoice closes: `[start, end)`, its committed days (paid trial days are not
/// committed days) per grid period against that period's days, and whether a termination cut it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClosedPeriod {
    pub start: NaiveDate,
    pub end: NaiveDate,
    /// More than one when a renewal settles periods an earlier release left unbilled.
    pub served: Vec<(i64, i64)>,
    pub cut_short: bool,
}

impl ClosedPeriod {
    /// The minimum over the period (`billing_core::minimum::floor`).
    pub(crate) fn floor(&self, minimum: i64) -> i64 {
        core::floor(minimum, &self.served)
    }
}

impl PeriodTrueUp {
    /// The minimum prorated to the days served.
    pub fn floor(&self) -> i64 {
        self.true_up.floor
    }

    /// `prior` as its drafts will be issued, the invoice closing the period taking a use of each
    /// of `consumed`: each draft discounted by the coupons it is owed, oldest period first (R-022).
    pub(crate) fn prior_as_issued(
        &self,
        coupons: &[AppliedCouponDetailed],
        currency: &str,
        consumed: &[AppliedCouponId],
    ) -> Vec<Spend> {
        let mut prior = self.prior.clone();
        if self.provisional.is_empty() {
            return prior;
        }
        let taken = self.queue.taken(coupons, currency, consumed);
        for draft in &self.provisional {
            let Some(owed) = taken.get(&draft.id) else {
                continue;
            };
            let same =
                owed.len() == draft.shown.len() && owed.iter().all(|id| draft.shown.contains(id));
            if same {
                continue;
            }
            let kept: Vec<&AppliedCouponDetailed> = coupons
                .iter()
                .filter(|coupon| owed.contains(&coupon.applied_coupon.id))
                .collect();
            let off: i64 = billing_core::assembly::discount::calculate_coupons_discount(
                draft.quoted.iter().sum(),
                currency,
                &super::discount::coupon_inputs(&kept),
            )
            .iter()
            .map(|applied| applied.value)
            .sum();
            let taxable = billing_core::assembly::discount::distribute_discount(
                &draft.quoted,
                u64::try_from(off).unwrap_or(0),
            );
            for (index, at) in &draft.lines {
                let (Some(spend), Some(quoted)) = (prior.get_mut(*index), draft.quoted.get(*at))
                else {
                    continue;
                };
                let share = quoted - taxable.get(*at).copied().flatten().unwrap_or(*quoted);
                spend.net += (quoted - spend.net).max(0) - share;
            }
        }
        prior
    }
}

/// A stored line as the minimum reads it: net of its coupons, gross of promotional credit, in
/// the figures it was priced in (tax included on a tax-inclusive line); `corrected` what notes
/// correcting it took back, read against its stored subtotal.
fn spend_of(line: &LineItem, tax_inclusive: bool, corrected: i64, voided: bool) -> Spend {
    let promotional = line.promotional_credit.unwrap_or(0);
    let net = if tax_inclusive {
        line.amount_total + promotional
    } else {
        line.taxable_amount + promotional
    };
    Spend {
        product: line.product_id,
        from: line.start_date,
        until: line.end_date,
        subtotal: line.amount_subtotal,
        net,
        corrected,
        fixed: line.metric_id.is_none(),
        voided,
        nominal: None,
    }
}

/// A line of the invoice being assembled, its coupons spread over it (`assemble_lines`) and its
/// tax not resolved yet: its taxable amount is its net, in the figures it was priced in.
fn assembled_spend(line: &LineItem) -> Spend {
    Spend {
        product: line.product_id,
        from: line.start_date,
        until: line.end_date,
        subtotal: line.amount_subtotal,
        net: line.taxable_amount + line.promotional_credit.unwrap_or(0),
        corrected: 0,
        fixed: line.metric_id.is_none(),
        voided: false,
        nominal: None,
    }
}

fn core_scope(scope: &MinimumScope) -> core::Scope {
    match scope {
        MinimumScope::AllComponents => core::Scope::AllComponents,
        MinimumScope::Products(ids) => core::Scope::Products(ids.clone()),
    }
}

/// The period an invoice dated `invoice_date` closes, `None` on an invoice that opens the
/// subscription's first period or when no committed day of it was served. After a change of
/// cadence, `run` gives the grid: the period is what its arrears settle.
pub(crate) fn closed_period(
    details: &SubscriptionDetails,
    invoice_date: NaiveDate,
    cycle_index: u32,
    cadence: &BillingPeriodEnum,
    run: Option<&crate::utils::periods::CadenceRun>,
) -> Option<ClosedPeriod> {
    use crate::domain::Period;
    use crate::utils::periods::{
        calculate_arrear_period_range, find_period_containing_date, subtract_months_at_billing_day,
    };
    let sub = &details.subscription;
    if !closes_a_period(details, cycle_index) {
        return None;
    }
    let billing_start = sub.billing_start_date?;
    let anchor = u32::from(sub.billing_day_anchor);
    let settles_early = sub.current_period_end.is_none();
    // On the subscription's grid (R-084): a change of cadence's, else the base grid from the
    // billing start or a free trial's end. A closing date off it (a history no rule produced)
    // keeps the period holding its day before.
    let months = cadence.as_months();
    let fee_run = match run {
        Some(run) => crate::utils::periods::FeeRun::of_cadence_run(run, months, anchor),
        None => crate::utils::periods::FeeRun::from_start(
            (
                billing_start,
                crate::services::invoice_lines::invoice_lines::billing_from(details, billing_start),
            ),
            anchor,
            months,
            invoice_date,
        ),
    }?;
    let mut windows: Vec<(Period, Period)> = fee_run.arrear_windows(invoice_date, settles_early);
    if windows.is_empty() && run.is_some() && !settles_early {
        log::warn!(
            "the period closed on {invoice_date} of subscription {} is off its cadence's grid",
            sub.id
        );
    }
    if windows.is_empty() && run.is_none() && !fee_run.grid.is_boundary(invoice_date) {
        log::warn!(
            "the period closed on {invoice_date} of subscription {} is off its grid",
            sub.id
        );
        let period = calculate_arrear_period_range(invoice_date, billing_start, anchor, cadence);
        let grid =
            find_period_containing_date(billing_start, invoice_date.pred_opt()?, cadence, anchor);
        let nominal_start =
            subtract_months_at_billing_day(grid.end, months, anchor).unwrap_or(grid.start);
        windows = vec![(
            period,
            Period {
                start: nominal_start,
                end: grid.end,
            },
        )];
    }
    let trial = paid_trial(details, billing_start);
    let served: Vec<(i64, i64)> = windows
        .iter()
        .filter(|(window, _)| window.start < window.end)
        .map(|(window, nominal)| {
            let trial_days = trial.map_or(0, |(from, to)| {
                (window.end.min(to) - window.start.max(from))
                    .num_days()
                    .max(0)
            });
            (
                (window.end - window.start).num_days() - trial_days,
                (nominal.end - nominal.start).num_days(),
            )
        })
        .filter(|(days, _)| *days > 0)
        .collect();
    let (first, last) = (windows.first()?, windows.last()?);
    if served.is_empty() {
        return None;
    }
    Some(ClosedPeriod {
        start: first.0.start,
        end: last.0.end,
        served,
        cut_short: settles_early && invoice_date < last.1.end,
    })
}

/// The days of a paid trial, from the billing start (its trial fact ends after as many days).
fn paid_trial(
    details: &SubscriptionDetails,
    billing_start: NaiveDate,
) -> Option<(NaiveDate, NaiveDate)> {
    let days = details.subscription.trial_duration?;
    details
        .trial_config
        .as_ref()
        .filter(|trial| !trial.is_free)?;
    Some((
        billing_start,
        billing_start + chrono::Duration::days(i64::from(days)),
    ))
}

/// Whether the subscription's invoice of `cycle_index` bills an elapsed period: a renewal, or an
/// early settlement (end date, termination). The first period's opening invoice never does.
fn closes_a_period(details: &SubscriptionDetails, cycle_index: u32) -> bool {
    let sub = &details.subscription;
    !sub.pending_checkout && (cycle_index > 0 || sub.current_period_end.is_none())
}

/// Loads the true-up of the period the Recurring invoice dated `invoice_date` closes, `None` when
/// it closes none or the period ran without a minimum.
pub(in crate::services) async fn load_period_true_up(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    invoice_date: NaiveDate,
    cycle_index: u32,
    invoice: Option<&crate::domain::Invoice>,
) -> StoreResult<Option<PeriodTrueUp>> {
    load_period_true_up_priced(
        conn,
        details,
        invoice_date,
        cycle_index,
        invoice,
        &Nominals::none(),
    )
    .await
}

/// [`load_period_true_up`], each prorated fixed fee line read over the whole period `nominals`
/// prices it on.
pub(in crate::services) async fn load_period_true_up_priced(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    invoice_date: NaiveDate,
    cycle_index: u32,
    invoice: Option<&crate::domain::Invoice>,
    nominals: &Nominals,
) -> StoreResult<Option<PeriodTrueUp>> {
    let sub = &details.subscription;
    if !closes_a_period(details, cycle_index) {
        return Ok(None);
    }
    let (minimum, cadence) = match outgoing_terms(conn, details, invoice_date).await? {
        Terms::Current => (details.minimum.clone(), sub.period),
        Terms::Outgoing(minimum, cadence) => (minimum, cadence),
        Terms::Unknown => {
            log::warn!(
                "subscription {}: the terms of the period ending {invoice_date} cannot be rebuilt; its minimum is not trued up",
                sub.id
            );
            return Ok(None);
        }
    };
    let Some(minimum) = minimum else {
        return Ok(None);
    };
    if minimum.amount_subunit < 0 {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Minimum amount_subunit must be non-negative, got {}",
            minimum.amount_subunit
        ))));
    }
    let run = super::cadence::load_cadence_run(conn, details, invoice_date, false).await?;
    let Some(period) = closed_period(details, invoice_date, cycle_index, &cadence, run.as_ref())
    else {
        return Ok(None);
    };
    let floor = period.floor(minimum.amount_subunit);
    if floor <= 0 {
        return Ok(None);
    }
    let (start, end) = (period.start, period.end);

    // A cancellation gives the unused days of what spans it back: only the days served count, a
    // longer fee's past an ending on the period's end too.
    let ended_here =
        period.cut_short || (sub.current_period_end.is_none() && sub.current_period_start == end);
    let cut = (sub.status == crate::domain::SubscriptionStatusEnum::Cancelled && ended_here)
        .then_some(end);
    let mut prior = Vec::new();
    let mut provisional = Vec::new();
    let mut up_front: Vec<(NaiveDate, NaiveDate, i64)> = Vec::new();
    let mut trued_up = 0;
    // A period a backdated ending cut short was billed past the cut too: its closing invoice's
    // arrears and true-up.
    let through = if period.cut_short {
        NaiveDate::MAX
    } else {
        end
    };
    let rows = diesel_models::invoices::InvoiceRow::list_period_spend_invoices(
        conn,
        sub.tenant_id,
        sub.id,
        start,
        through,
        invoice.map(|invoice| invoice.id),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    for row in rows {
        let invoice: crate::domain::Invoice = row.try_into()?;
        // A copy reissuing an invoice is read with the invoice it corrects.
        if invoice.parent_invoice_id.is_some() {
            continue;
        }
        let (lines, corrected) = period_lines(conn, sub.id, &invoice).await?;
        let voided = invoice.status == crate::domain::InvoiceStatusEnum::Void
            || merged_into_void(conn, sub.tenant_id, &invoice).await?;
        // A copy bills what its parent's correction took back; under a goodwill note of an
        // earlier release the parent still counts as billed and its copy does not.
        if corrected.values().any(|amount| *amount > 0)
            && let Some(copy) = reissued_copy(conn, sub.tenant_id, invoice.id).await?
        {
            let (copy_lines, copy_corrected) = period_lines(conn, sub.id, &copy).await?;
            for line in &copy_lines {
                let voided = copy.status == crate::domain::InvoiceStatusEnum::Void;
                if !voided && copy.status != crate::domain::InvoiceStatusEnum::Draft {
                    trued_up += true_up_billed(line, &copy, &copy_corrected, start, end);
                }
                prior.extend(regular_spend(
                    line,
                    &copy,
                    &copy_corrected,
                    voided,
                    (start, end),
                    nominals,
                ));
            }
        }
        let quote = |line: &LineItem| {
            if invoice.tax_inclusive {
                line.clone().into_tax_inclusive_quote().amount_subtotal
            } else {
                line.amount_subtotal
            }
        };
        let discounted: Vec<&LineItem> = invoice
            .line_items
            .iter()
            .filter(|line| line.kind != LineItemKind::MinimumShortfall)
            .collect();
        let mut draft = (!voided && invoice.status == crate::domain::InvoiceStatusEnum::Draft)
            .then(|| Provisional {
                id: invoice.id,
                shown: invoice
                    .coupons
                    .iter()
                    .map(|coupon| coupon.applied_coupon_id)
                    .collect(),
                quoted: discounted.iter().map(|line| quote(line)).collect(),
                lines: Vec::new(),
            });
        for line in &lines {
            if line.start_date < start || line.start_date >= end {
                continue;
            }
            if line.kind == LineItemKind::MinimumShortfall
                && is_up_front(line, invoice.billing_period_start)
            {
                let quoted = if invoice.tax_inclusive {
                    line.clone().into_tax_inclusive_quote().amount_subtotal
                } else {
                    line.amount_subtotal
                };
                up_front.push((line.start_date, line.end_date, quoted));
            }
            // A draft is no issued document: its true-up is computed again when it is.
            if !voided && invoice.status != crate::domain::InvoiceStatusEnum::Draft {
                trued_up += true_up_billed(line, &invoice, &corrected, start, end);
            }
            if let Some(spend) =
                regular_spend(line, &invoice, &corrected, voided, (start, end), nominals)
            {
                if let Some(draft) = draft.as_mut()
                    && let Some(at) = discounted
                        .iter()
                        .position(|other| other.local_id == line.local_id)
                {
                    draft.lines.push((prior.len(), at));
                }
                prior.push(spend);
            }
        }
        provisional.extend(draft.filter(|draft| !draft.lines.is_empty()));
    }
    let queue = if provisional.is_empty() {
        Queue::default()
    } else {
        Queue::load(conn, details, invoice.map(|i| i.id), invoice_date).await?
    };
    let closing_carries_one = invoice.is_some_and(|invoice| {
        invoice
            .line_items
            .iter()
            .any(|line| is_up_front(line, invoice_date))
    });
    let up_front_rule = if up_front.is_empty() && !closing_carries_one {
        None
    } else {
        let scope = details
            .minimum
            .as_ref()
            .map_or(&minimum.scope, |m| &m.scope);
        Some(load_up_front_shortfall(conn, sub.tenant_id, sub.id, scope).await?)
    };
    let prepaid = match up_front_rule {
        Some(UpFrontShortfall::Prepaid) => up_front,
        _ => Vec::new(),
    };
    let true_up = core::TrueUp {
        start,
        end,
        floor,
        scope: core_scope(&minimum.scope),
        cut,
        prepaid,
        trued_up,
        carried: carried_scope_products(details, &minimum.scope),
    };

    Ok(Some(PeriodTrueUp {
        start,
        end,
        minimum,
        true_up,
        prior,
        up_front: up_front_rule,
        provisional,
        queue,
    }))
}

/// The true-ups of the periods before `before` whose spend an ending on `ended` gives back, a
/// line of theirs billing past it: each period closed by an issued invoice, its lines counted as
/// served up to the ending, what it was trued up with deducted (R-022). Never credited back.
pub(in crate::services) async fn load_reopened_true_ups(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    before: NaiveDate,
    ended: NaiveDate,
    nominals: &Nominals,
) -> StoreResult<Vec<PeriodTrueUp>> {
    use crate::domain::InvoiceStatusEnum;
    let sub = &details.subscription;
    let rows = diesel_models::invoices::InvoiceRow::list_period_spend_invoices(
        conn,
        sub.tenant_id,
        sub.id,
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap_or(before),
        before.succ_opt().unwrap_or(before),
        None,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let invoices: Vec<crate::domain::Invoice> = rows
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<_, _>>()?;
    let cut_from: Vec<NaiveDate> = invoices
        .iter()
        .filter(|invoice| {
            invoice.status != InvoiceStatusEnum::Void && invoice.subscription_id == Some(sub.id)
        })
        .flat_map(|invoice| &invoice.line_items)
        .filter(|line| {
            line.kind == LineItemKind::Regular && line.start_date < before && line.end_date > ended
        })
        .map(|line| line.start_date)
        .collect();
    // The invoice closing the period holding each: the first renewal after its start.
    let mut closing: Vec<&crate::domain::Invoice> = Vec::new();
    for from in cut_from {
        let next = invoices
            .iter()
            .filter(|invoice| {
                invoice.invoice_type == InvoiceType::Recurring
                    && invoice.parent_invoice_id.is_none()
                    && invoice.billing_period_start > from
                    && invoice.billing_period_start <= before
            })
            .min_by_key(|invoice| invoice.billing_period_start);
        if let Some(invoice) = next.filter(|invoice| {
            matches!(
                invoice.status,
                InvoiceStatusEnum::Finalized | InvoiceStatusEnum::Uncollectible
            )
        }) && !closing.iter().any(|known| known.id == invoice.id)
        {
            closing.push(invoice);
        }
    }
    let mut reopened = Vec::new();
    for invoice in closing {
        // The period as its renewal closed it, the subscription still running then.
        let mut running = details.clone();
        running.subscription.current_period_end = Some(invoice.billing_period_start);
        let cycle_index = invoice.cycle_index.unwrap_or(1).max(1);
        let Some(mut true_up) = load_period_true_up_priced(
            conn,
            &running,
            invoice.billing_period_start,
            cycle_index,
            Some(invoice),
            nominals,
        )
        .await?
        else {
            continue;
        };
        let (start, end) = (true_up.start, true_up.end);
        let (lines, corrected) = period_lines(conn, sub.id, invoice).await?;
        for line in &lines {
            true_up.true_up.trued_up += true_up_billed(line, invoice, &corrected, start, end);
            true_up.prior.extend(regular_spend(
                line,
                invoice,
                &corrected,
                false,
                (start, end),
                nominals,
            ));
        }
        true_up.true_up.cut = Some(ended);
        reopened.push(true_up);
    }
    Ok(reopened)
}

/// `invoice`'s lines of subscription `subscription_id`, with what notes correcting them took back.
async fn period_lines(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    invoice: &crate::domain::Invoice,
) -> StoreResult<(Vec<LineItem>, HashMap<String, i64>)> {
    let mut lines = invoice.line_items.clone();
    if invoice.subscription_id.is_none() {
        let owners =
            crate::services::credit::scope::LineOwners::load_tx(conn, invoice.tenant_id, &lines)
                .await?;
        lines.retain(|line| {
            crate::services::credit::scope::line_subscription(line, None, &owners)
                == Some(subscription_id)
        });
    }
    let (_, _, corrected, _) =
        crate::services::subscriptions::amendment::load_credit_matching_context(
            conn,
            invoice.tenant_id,
            invoice,
        )
        .await?;
    Ok((lines, corrected))
}

/// The live copy reissuing `parent`, if any.
async fn reissued_copy(
    conn: &mut PgConn,
    tenant_id: TenantId,
    parent: common_domain::ids::InvoiceId,
) -> StoreResult<Option<crate::domain::Invoice>> {
    let db = |e| Into::<Report<StoreError>>::into(e);
    let Some(id) =
        diesel_models::invoices::InvoiceRow::find_child_id_by_parent(conn, tenant_id, parent)
            .await
            .map_err(db)?
    else {
        return Ok(None);
    };
    let row = diesel_models::invoices::InvoiceRow::find_by_id(conn, tenant_id, id)
        .await
        .map_err(db)?;
    if !matches!(
        row.status,
        diesel_models::enums::InvoiceStatusEnum::Draft
            | diesel_models::enums::InvoiceStatusEnum::Finalized
            | diesel_models::enums::InvoiceStatusEnum::Uncollectible
    ) {
        return Ok(None);
    }
    Ok(Some(row.try_into()?))
}

/// A fixed fee charge's lineage and key period, its whole period and price, for seats one seat's
/// price, and the first day the charge bills.
pub(in crate::services) type NominalPeriod = (
    Fee,
    (NaiveDate, NaiveDate),
    core::Nominal,
    Option<i64>,
    NaiveDate,
);

/// The whole periods the facts price the subscription's fixed fees over, with their prices and,
/// for seats, one seat's.
pub(in crate::services) struct Nominals {
    lineages: Lineages,
    periods: Vec<NominalPeriod>,
}

impl Nominals {
    /// `periods`: each fixed fee charge's lineage and key period, with its nominal period and the
    /// price of a whole one.
    pub(in crate::services) fn new(lineages: Lineages, periods: Vec<NominalPeriod>) -> Self {
        Nominals { lineages, periods }
    }

    fn none() -> Self {
        Nominals::new(Lineages::of_roots([]), Vec::new())
    }

    /// The whole period and price `line` prorates, when they price it to its subtotal.
    fn of(&self, line: &LineItem) -> Option<core::Nominal> {
        if line.metric_id.is_some() || line.credit_unit.is_some() {
            return None;
        }
        let key = evaluated_charge_key(line, &self.lineages);
        if key.kind != ChargeKind::Fee {
            return None;
        }
        let held = |nominal: &core::Nominal| DaySpan {
            period: nominal.period,
            from: line.start_date,
            until: line.end_date,
        };
        let covering = || {
            self.periods.iter().filter(|(fee, (from, until), _, _, _)| {
                *fee == key.fee && *from <= line.start_date && line.end_date <= *until
            })
        };
        if let Some(nominal) = covering()
            .map(|(_, _, nominal, _, _)| *nominal)
            .find(|nominal| held(nominal).prorate(nominal.full) == line.amount_subtotal)
        {
            return Some(nominal);
        }
        // Seats bought inside the period: what is left at the count after them less at the
        // count before (R-079): the step's own charge's count, else read back from its amount.
        let bought = line
            .quantity
            .filter(|quantity| quantity.fract().is_zero())
            .and_then(|quantity| quantity.to_i64())
            .filter(|bought| *bought > 0)?;
        let step = |nominal: &core::Nominal, seat: i64, after: i64| {
            let step = core::Nominal {
                full: after * seat,
                base: (after - bought) * seat,
                ..*nominal
            };
            let span = held(&step);
            (span.prorate(step.full) - span.prorate(step.base) == line.amount_subtotal)
                .then_some(step)
        };
        let seat_of = |seat: &Option<i64>| (*seat).filter(|seat| *seat > 0);
        covering()
            .filter(|(_, _, nominal, seat, from)| {
                *from == line.start_date
                    && seat_of(seat).is_some_and(|seat| nominal.full % seat == 0)
            })
            .find_map(|(_, _, nominal, seat, _)| {
                let seat = seat_of(seat)?;
                Some(nominal.full / seat)
                    .filter(|after| *after > bought)
                    .and_then(|after| step(nominal, seat, after))
            })
            .or_else(|| {
                covering().find_map(|(_, _, nominal, seat, _)| {
                    let seat = seat_of(seat)?;
                    (bought + 1..=nominal.full / seat).find_map(|after| step(nominal, seat, after))
                })
            })
    }
}

/// A regular line of `invoice` dated in `[start, end)` as the minimum reads it.
fn regular_spend(
    line: &LineItem,
    invoice: &crate::domain::Invoice,
    corrected: &HashMap<String, i64>,
    voided: bool,
    (start, end): (NaiveDate, NaiveDate),
    nominals: &Nominals,
) -> Option<Spend> {
    (line.kind == LineItemKind::Regular && line.start_date >= start && line.start_date < end).then(
        || Spend {
            nominal: nominals.of(line),
            ..spend_of(
                line,
                invoice.tax_inclusive,
                corrected.get(&line.local_id).copied().unwrap_or(0),
                voided,
            )
        },
    )
}

/// What `line` of `invoice` billed as a true-up of the period `[start, end)`, net of its notes,
/// in the figures the floor is in.
fn true_up_billed(
    line: &LineItem,
    invoice: &crate::domain::Invoice,
    corrected: &HashMap<String, i64>,
    start: NaiveDate,
    end: NaiveDate,
) -> i64 {
    if line.kind != LineItemKind::MinimumShortfall
        || is_up_front(line, invoice.billing_period_start)
        || line.start_date < start
        || line.start_date >= end
    {
        return 0;
    }
    let billed = if invoice.tax_inclusive {
        line.clone().into_tax_inclusive_quote().amount_subtotal
    } else {
        line.amount_subtotal
    };
    let corrected = corrected.get(&line.local_id).copied().unwrap_or(0);
    if corrected == 0 || line.amount_subtotal == 0 {
        return billed;
    }
    let share = billing_core::rounding::div_round(
        i128::from(corrected) * i128::from(billed),
        i128::from(line.amount_subtotal),
    );
    billed - i64::try_from(share).unwrap_or(billed)
}

/// Whether `invoice` was merged into a consolidated invoice since voided: voided with it.
async fn merged_into_void(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice: &crate::domain::Invoice,
) -> StoreResult<bool> {
    let Some(parent) = invoice.consolidated_into_invoice_id else {
        return Ok(false);
    };
    let row = diesel_models::invoices::InvoiceRow::find_by_id(conn, tenant_id, parent)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(row.status == diesel_models::enums::InvoiceStatusEnum::Void)
}

/// A shortfall the rule before the true-up charged up front: dated past its invoice's period start.
pub fn is_up_front(line: &LineItem, billing_period_start: NaiveDate) -> bool {
    line.kind == LineItemKind::MinimumShortfall && line.end_date > billing_period_start
}

/// That rule's floor applied on the opening invoice when it billed an in-scope fee there (only
/// advance and one-time fees bill on it, zero-width lines included) or charged a shortfall.
pub(crate) fn up_front_shortfall_of(
    opening: &[LineItem],
    scope: &MinimumScope,
) -> UpFrontShortfall {
    let applied = opening
        .iter()
        .any(|line| line.kind == LineItemKind::MinimumShortfall)
        || counted_lines(opening, scope).any(|line| line.metric_id.is_none());
    if applied {
        UpFrontShortfall::Prepaid
    } else {
        UpFrontShortfall::TrueUpOfPeriodBefore
    }
}

/// How the rule before the true-up dated the subscription's shortfalls, read on its opening invoice
/// against the minimum's `scope`.
pub(crate) async fn load_up_front_shortfall(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    scope: &MinimumScope,
) -> StoreResult<UpFrontShortfall> {
    let opening = diesel_models::invoices::InvoiceRow::find_opening_recurring_invoice(
        conn,
        tenant_id,
        subscription_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let lines: Vec<LineItem> = match opening {
        Some(row) => {
            let invoice: crate::domain::Invoice = row.try_into()?;
            invoice
                .line_items
                .into_iter()
                .filter(|line| line.subscription_id.is_none_or(|id| id == subscription_id))
                .collect()
        }
        None => Vec::new(),
    };
    Ok(up_front_shortfall_of(&lines, scope))
}

/// Whether a shortfall the subscription's invoices charged up front prepaid its period; never
/// without a minimum, whose shortfalls stay as billed.
pub(crate) async fn up_front_shortfalls_prepaid(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
) -> StoreResult<bool> {
    let Some(minimum) = &details.minimum else {
        return Ok(false);
    };
    let sub = &details.subscription;
    Ok(
        load_up_front_shortfall(conn, sub.tenant_id, sub.id, &minimum.scope).await?
            == UpFrontShortfall::Prepaid,
    )
}

/// The rule of each subscription whose `invoices` (tenant, subscription, period start, lines) carry
/// a shortfall charged up front; one without a minimum any more is left out.
pub async fn load_up_front_shortfalls<'a>(
    conn: &mut PgConn,
    invoices: impl Iterator<Item = (TenantId, Option<SubscriptionId>, NaiveDate, &'a [LineItem])>,
) -> StoreResult<HashMap<SubscriptionId, UpFrontShortfall>> {
    let subscriptions: std::collections::HashSet<(TenantId, SubscriptionId)> = invoices
        .filter(|(_, _, start, lines)| lines.iter().any(|line| is_up_front(line, *start)))
        .filter_map(|(tenant_id, subscription_id, _, _)| Some((tenant_id, subscription_id?)))
        .collect();
    let mut rules = HashMap::new();
    for (tenant_id, subscription_id) in subscriptions {
        let Some(minimum) =
            crate::repositories::subscriptions::load_subscription_minimum(conn, subscription_id)
                .await?
        else {
            continue;
        };
        let rule =
            load_up_front_shortfall(conn, tenant_id, subscription_id, &minimum.scope).await?;
        rules.insert(subscription_id, rule);
    }
    Ok(rules)
}

enum Terms {
    /// No plan change at the period's end: the subscription's terms.
    Current,
    /// The minimum and cadence a plan change at the period's end replaced.
    Outgoing(Option<SubscriptionMinimum>, BillingPeriodEnum),
    /// A plan change at the period's end whose outgoing terms cannot be read back.
    Unknown,
}

/// The terms a period ran under when a plan change applied at its end replaced them: as the
/// change recorded them, else rebuilt from the plan version it left and the components it closed.
async fn outgoing_terms(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    period_end: NaiveDate,
) -> StoreResult<Terms> {
    #[derive(serde::Deserialize)]
    struct Outgoing {
        amount_subunit: i64,
        includes_all_components: bool,
        product_ids: Vec<ProductId>,
    }
    let subscription_id = details.subscription.id;
    let events =
        diesel_models::subscription_events::SubscriptionEventRow::fetch_by_subscription_id_and_date(
            conn,
            subscription_id,
            period_end,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    // Recorded as the typed id's string (base62, or a UUID in older rows): `FromStr` reads both.
    let plan_of = |details: &serde_json::Value, key: &str| {
        details
            .get(key)
            .and_then(serde_json::Value::as_str)
            .and_then(|id| id.parse::<common_domain::ids::PlanVersionId>().ok())
    };
    let Some(switch) = events
        .into_iter()
        .filter(|event| {
            matches!(
                event.event_type,
                diesel_models::enums::SubscriptionEventType::Switch
            )
        })
        .filter_map(|event| event.details)
        .find(|details| {
            plan_of(details, "old_plan_version_id").is_some()
                && plan_of(details, "old_plan_version_id")
                    != plan_of(details, "new_plan_version_id")
        })
    else {
        return Ok(Terms::Current);
    };

    if switch.get(OUTGOING_MINIMUM_KEY).is_some() {
        fn parse<T: serde::de::DeserializeOwned>(
            details: &serde_json::Value,
            key: &str,
        ) -> StoreResult<T> {
            serde_json::from_value(details.get(key).cloned().unwrap_or_default()).map_err(|error| {
                Report::new(StoreError::SerdeError(format!("switch event {key}"), error))
            })
        }
        let outgoing: Option<Outgoing> = parse(&switch, OUTGOING_MINIMUM_KEY)?;
        let cadence: BillingPeriodEnum = parse(&switch, OUTGOING_PERIOD_KEY)?;
        return Ok(Terms::Outgoing(
            outgoing.map(|o| SubscriptionMinimum {
                subscription_id,
                amount_subunit: o.amount_subunit,
                scope: if o.includes_all_components {
                    MinimumScope::AllComponents
                } else {
                    MinimumScope::Products(o.product_ids)
                },
                is_override: false,
            }),
            cadence,
        ));
    }

    // Written before the change recorded them: an override survived the change, a plan snapshot
    // was the version's; the cadence is the shortest of the components the change closed.
    let Some(old_version) = plan_of(&switch, "old_plan_version_id") else {
        return Ok(Terms::Unknown);
    };
    let minimum = match details.minimum.as_ref().filter(|m| m.is_override) {
        Some(minimum) => Some(minimum.clone()),
        None => crate::repositories::plans::load_plan_minimum(conn, old_version)
            .await?
            .map(|minimum| SubscriptionMinimum {
                subscription_id,
                amount_subunit: minimum.amount_subunit,
                scope: minimum.scope,
                is_override: false,
            }),
    };
    let closed = diesel_models::subscription_components::SubscriptionComponentRow::list_component_history_for_period(
        conn,
        &subscription_id,
        period_end.pred_opt().unwrap_or(period_end),
        period_end,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let mut cadence: Option<BillingPeriodEnum> = None;
    for row in closed
        .into_iter()
        .filter(|row| row.effective_to == Some(period_end))
    {
        let component: crate::domain::SubscriptionComponent = row.try_into()?;
        if let Some(period) = component.period.as_billing_period_opt() {
            cadence = Some(cadence.map_or(period, |c| {
                if period.as_months() < c.as_months() {
                    period
                } else {
                    c
                }
            }));
        }
    }
    Ok(match cadence {
        Some(cadence) => Terms::Outgoing(minimum, cadence),
        None => Terms::Unknown,
    })
}

/// The record a plan change applied at a period's end keeps of the terms that period ran under.
pub(crate) fn outgoing_terms_record(
    minimum: Option<&SubscriptionMinimum>,
    cadence: BillingPeriodEnum,
) -> [(&'static str, serde_json::Value); 2] {
    let minimum = minimum.map(|m| {
        let (includes_all_components, product_ids) = match &m.scope {
            MinimumScope::AllComponents => (true, vec![]),
            MinimumScope::Products(ids) => (false, ids.clone()),
        };
        serde_json::json!({
            "amount_subunit": m.amount_subunit,
            "includes_all_components": includes_all_components,
            "product_ids": product_ids,
        })
    });
    [
        (
            OUTGOING_MINIMUM_KEY,
            minimum.unwrap_or(serde_json::Value::Null),
        ),
        (
            OUTGOING_PERIOD_KEY,
            serde_json::to_value(cadence).unwrap_or_default(),
        ),
    ]
}

/// Drops the true-up lines an earlier computation of the invoice appended, so a refresh
/// recomputes them; any other shortfall line stays as billed.
pub fn drop_true_up_lines(
    invoice_type: &InvoiceType,
    lines: &mut Vec<LineItem>,
    invoice_date: NaiveDate,
    true_up: Option<&PeriodTrueUp>,
) {
    // A final invoice holds a true-up only when it stands for its ending day's recurring one.
    if matches!(invoice_type, InvoiceType::Recurring | InvoiceType::Final) {
        lines.retain(|l| !is_true_up(l, invoice_date, true_up));
    }
}

/// Appends the true-up of the period the invoice closes when its in-scope spend falls short of
/// the prorated minimum, to `lines` its coupons were already spread over: the shortfall is never
/// discounted (R-023), its amount `billing_core::minimum`'s.
pub fn append_minimum_shortfall_if_any(
    invoice_type: &InvoiceType,
    subscription_details: &SubscriptionDetails,
    lines: &mut Vec<LineItem>,
    true_up: Option<&PeriodTrueUp>,
    consumed: &[AppliedCouponId],
) -> StoreResult<()> {
    if !matches!(invoice_type, InvoiceType::Recurring | InvoiceType::Final) {
        return Ok(());
    }
    let Some(true_up) = true_up else {
        return Ok(());
    };
    let scope = &true_up.minimum.scope;
    let document: Vec<Spend> = lines
        .iter()
        .filter(|line| line.kind == LineItemKind::Regular)
        .map(assembled_spend)
        .collect();
    let prior = true_up.prior_as_issued(
        &subscription_details.applied_coupons,
        &subscription_details.subscription.currency,
        consumed,
    );
    let split = true_up.true_up.lines(&document, &prior);
    if split.is_empty() {
        return Ok(());
    }

    let sub = &subscription_details.subscription;
    // For a product-scoped minimum, name the covered product(s) on the line so it's clear which
    // commitment is being trued up (e.g. "Minimum commitment (Bandwidth)").
    let name = minimum_line_name(subscription_details, scope);
    let named_per_product = split.len() > 1;
    for (product_id, amount) in split {
        let name = match product_id.filter(|_| named_per_product) {
            Some(product) => format!(
                "{name} · {}",
                product_name(subscription_details, product).unwrap_or_default()
            ),
            None => name.clone(),
        };
        lines.push(shortfall_line(
            name,
            amount,
            product_id,
            true_up.start,
            true_up.end,
            sub.id,
        ));
    }

    Ok(())
}

/// A shortfall line this invoice trues up a period with, recomputed on every refresh: one dated
/// by the invoice's date, or one past it the rule before the true-up charged as the true-up of
/// `true_up`'s period, which replaces it. Any other stays as billed.
pub(crate) fn is_true_up(
    line: &LineItem,
    invoice_date: NaiveDate,
    true_up: Option<&PeriodTrueUp>,
) -> bool {
    line.kind == LineItemKind::MinimumShortfall
        && (line.end_date <= invoice_date
            || true_up.is_some_and(|t| t.up_front == Some(UpFrontShortfall::TrueUpOfPeriodBefore)))
}

fn shortfall_line(
    name: String,
    amount: i64,
    product_id: Option<ProductId>,
    start_date: NaiveDate,
    end_date: NaiveDate,
    subscription_id: SubscriptionId,
) -> LineItem {
    let line = LineItem {
        local_id: String::new(),
        name,
        charge_type: Some(ChargeType::Recurring),
        amount_subtotal: amount,
        tax_rate: Decimal::ZERO,
        taxable_amount: amount,
        tax_amount: 0,
        amount_total: amount,
        tax_details: Vec::new(),
        quantity: None,
        unit_price: None,
        start_date,
        end_date,
        sub_lines: Vec::new(),
        is_prorated: false,
        price_component_id: None,
        sub_component_id: None,
        sub_add_on_id: None,
        product_id,
        metric_id: None,
        description: None,
        group_by_dimensions: None,
        kind: LineItemKind::MinimumShortfall,
        credit_grant_amount: None,
        credit_grant_expiry: None,
        credit_grant_included: false,
        taxed_at_purchase: false,
        tax_inclusive_quote: None,
        credit_unit: None,
        credit_grant_unit: None,
        credit_grant_rollover: None,
        promotional_credit: None,
        subscription_id: Some(subscription_id),
        usage_ends_at: None,
        usage_breakdown: None,
    };
    super::charges::identified(line, &super::charges::Lineages::none())
}

/// How a shortfall is billed (`billing_core::minimum::split`).
pub(crate) fn split_shortfall(
    scope: &MinimumScope,
    weights: &[(ProductId, i64)],
    carried: &[ProductId],
    shortfall: i64,
) -> Vec<(Option<ProductId>, i64)> {
    core::split(&core_scope(scope), weights, carried, shortfall)
}

/// The scoped products the subscription's components and add-ons are sold under, in scope order.
fn carried_scope_products(
    subscription_details: &SubscriptionDetails,
    scope: &MinimumScope,
) -> Vec<ProductId> {
    let MinimumScope::Products(ids) = scope else {
        return vec![];
    };
    let carried: Vec<ProductId> = subscription_details
        .price_components
        .iter()
        .filter_map(|c| c.product_id)
        .chain(
            subscription_details
                .add_ons
                .iter()
                .filter_map(|a| a.product_id),
        )
        .collect();
    ids.iter()
        .filter(|id| carried.contains(id))
        .copied()
        .collect()
}

fn product_name(subscription_details: &SubscriptionDetails, product: ProductId) -> Option<String> {
    subscription_details
        .price_components
        .iter()
        .find(|c| c.product_id == Some(product))
        .map(|c| c.name.clone())
        .or_else(|| {
            subscription_details
                .add_ons
                .iter()
                .find(|a| a.product_id == Some(product))
                .map(|a| a.name.clone())
        })
}

/// Display name for the shortfall line. `AllComponents` uses the bare label; a product-scoped
/// minimum appends the covered component name(s) — e.g. "Minimum commitment (Bandwidth)" — so the
/// invoice (and checkout preview) shows which commitment the top-up is for.
fn minimum_line_name(subscription_details: &SubscriptionDetails, scope: &MinimumScope) -> String {
    let names = scope_component_names(subscription_details, scope);
    if names.is_empty() {
        MINIMUM_COMMITMENT_LINE_NAME.to_string()
    } else {
        format!("{} ({})", MINIMUM_COMMITMENT_LINE_NAME, names.join(", "))
    }
}

/// Sorted names of the components and add-ons a product-scoped minimum covers; empty for
/// `AllComponents`.
fn scope_component_names(
    subscription_details: &SubscriptionDetails,
    scope: &MinimumScope,
) -> Vec<String> {
    let MinimumScope::Products(ids) = scope else {
        return vec![];
    };

    let mut names: Vec<String> = subscription_details
        .price_components
        .iter()
        .filter(|c| c.product_id.map(|p| ids.contains(&p)).unwrap_or(false))
        .map(|c| c.name.clone())
        .chain(
            subscription_details
                .add_ons
                .iter()
                .filter(|a| a.product_id.map(|p| ids.contains(&p)).unwrap_or(false))
                .map(|a| a.name.clone()),
        )
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The in-scope revenue lines: the opening invoice's for the rule before the true-up, and the
/// usage a projection extrapolates.
///
/// Scope rules (the synthesized shortfall line is excluded by `kind == Regular`):
/// - `AllComponents`: every subscription revenue line counts — plan components, overrides,
///   ad-hoc extras AND add-ons.
/// - `Products(ids)`: only lines whose `product_id` is in `ids`. A product is the identity shared
///   by plan components, overrides and extras (and carried on add-on lines), so a subscription's
///   actual billed set is matched uniformly even when it diverges from the plan. Legacy lines with
///   no product (`product_id = None`) can't be product-scoped and only count under `AllComponents`.
fn counted_lines<'a>(
    lines: &'a [LineItem],
    scope: &'a MinimumScope,
) -> impl Iterator<Item = &'a LineItem> {
    lines
        .iter()
        .filter(|l| l.kind == LineItemKind::Regular)
        .filter(move |l| match scope {
            MinimumScope::AllComponents => true,
            MinimumScope::Products(_) => match l.product_id {
                Some(id) => scope.includes(id),
                None => false,
            },
        })
}

/// End-of-period estimate of a subscription's spend against its minimum commitment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortfallProjection {
    pub floor_cents: i64,
    pub counted_to_date_cents: i64,
    pub projected_cents: i64,
    pub projected_shortfall_cents: i64,
    /// Covered component names; empty when the minimum spans all components.
    pub scope_product_names: Vec<String>,
}

/// Projects the shortfall of the period the upcoming invoice closes, from its `lines` and what
/// the period billed elsewhere, at `elapsed` (0..=1) of the period. Only lines of usage fees on
/// summed or counted metrics are extrapolated, linearly; everything else is final.
pub fn project_minimum_shortfall(
    invoice_details: &SubscriptionDetails,
    lines: &[LineItem],
    elapsed: f64,
    true_up: &PeriodTrueUp,
) -> Option<ShortfallProjection> {
    let scope = &true_up.minimum.scope;
    let document: Vec<Spend> = lines
        .iter()
        .filter(|line| line.kind == LineItemKind::Regular)
        .map(assembled_spend)
        .collect();
    let counted: i64 = true_up
        .true_up
        .spend(&document, &true_up.prior)
        .iter()
        .map(|(_, amount)| amount)
        .sum();
    let usage: i64 = counted_lines(lines, scope)
        .filter(|l| l.start_date >= true_up.start && l.start_date < true_up.end)
        .filter(|l| extrapolates(invoice_details, l))
        .map(|l| assembled_spend(l).net)
        .sum();
    project_shortfall(&true_up.true_up, counted, usage, elapsed).map(|p| ShortfallProjection {
        scope_product_names: scope_component_names(invoice_details, scope),
        ..p
    })
}

/// A line that grows with the period: a usage fee's line (a capacity fee's lines, commitment and
/// overage alike, are stamped `Usage` too) on a metric that sums or counts.
fn extrapolates(details: &SubscriptionDetails, line: &LineItem) -> bool {
    use crate::domain::enums::BillingMetricAggregateEnum as Aggregate;
    use crate::domain::subscription_components::SubscriptionFee;

    if line.charge_type != Some(ChargeType::Usage) {
        return false;
    }
    let fee = line
        .sub_component_id
        .and_then(|id| details.price_components.iter().find(|c| c.id == id))
        .map(|c| &c.fee)
        .or_else(|| {
            line.sub_add_on_id
                .and_then(|id| details.add_ons.iter().find(|a| a.id == id))
                .map(|a| &a.fee)
        });
    let Some(SubscriptionFee::Usage { metric_id, .. }) = fee else {
        return false;
    };
    details.metrics.iter().any(|m| {
        m.id == *metric_id && matches!(m.aggregation_type, Aggregate::Sum | Aggregate::Count)
    })
}

fn project_shortfall(
    true_up: &core::TrueUp,
    counted: i64,
    usage: i64,
    elapsed: f64,
) -> Option<ShortfallProjection> {
    let elapsed = Decimal::from_f64(elapsed)?.min(Decimal::ONE);
    if elapsed <= Decimal::ZERO {
        return None;
    }
    let usage = Decimal::from(usage);
    let remaining = (usage / elapsed - usage)
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()?;
    let projected = counted.saturating_add(remaining);
    Some(ShortfallProjection {
        floor_cents: true_up.floor,
        counted_to_date_cents: counted,
        projected_cents: projected,
        projected_shortfall_cents: true_up.shortfall(projected),
        scope_product_names: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use common_domain::ids::{BaseId, ProductId, SubscriptionAddOnId};

    /// A 100.00 shortfall on two products that counted 60.00 and 30.00 is billed 66.67 and 33.33
    /// on lines of their own; with nothing counted, it is shared equally; an all-components
    /// minimum keeps one line with no product (the default tax).
    #[test]
    fn a_product_scoped_shortfall_is_split_by_what_each_product_counted() {
        let (a, b) = (ProductId::new(), ProductId::new());
        let scope = MinimumScope::Products(vec![a, b]);
        assert_eq!(
            split_shortfall(
                &scope,
                &[(a, 4_000), (b, 3_000), (a, 2_000)],
                &[a, b],
                10_000
            ),
            vec![(Some(a), 6_667), (Some(b), 3_333)]
        );
        assert_eq!(
            split_shortfall(&scope, &[], &[b, a], 10_001),
            vec![(Some(b), 5_001), (Some(a), 5_000)]
        );
        assert_eq!(
            split_shortfall(&MinimumScope::AllComponents, &[(a, 1)], &[a], 500),
            vec![(None, 500)]
        );
    }

    fn line(
        amount: i64,
        product: Option<ProductId>,
        add_on: Option<SubscriptionAddOnId>,
        kind: LineItemKind,
    ) -> LineItem {
        LineItem {
            local_id: "x".into(),
            name: "l".into(),
            charge_type: None,
            amount_subtotal: amount,
            tax_rate: Decimal::ZERO,
            taxable_amount: amount,
            tax_amount: 0,
            amount_total: amount,
            tax_details: vec![],
            quantity: None,
            unit_price: None,
            start_date: NaiveDate::MIN,
            end_date: NaiveDate::MIN,
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: add_on,
            product_id: product,
            metric_id: None,
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

    fn counted_total(lines: &[LineItem], scope: &MinimumScope) -> i64 {
        counted_lines(lines, scope).map(|l| l.amount_subtotal).sum()
    }

    #[test]
    fn counted_total_all_components_includes_components_addons_and_extras() {
        let a = ProductId::new();
        let b = ProductId::new();
        let ao = SubscriptionAddOnId::new();
        let lines = vec![
            line(1000, Some(a), None, LineItemKind::Regular),
            line(500, Some(b), None, LineItemKind::Regular),
            line(700, Some(b), Some(ao), LineItemKind::Regular), // add-on
            line(2000, Some(a), None, LineItemKind::Regular),    // extra (no plan component)
            line(999, None, None, LineItemKind::MinimumShortfall),
        ];
        // Every revenue line counts under AllComponents (the shortfall line is excluded).
        assert_eq!(counted_total(&lines, &MinimumScope::AllComponents), 4200);
    }

    #[test]
    fn counted_total_specific_products_matches_by_product() {
        let a = ProductId::new();
        let b = ProductId::new();
        let c = ProductId::new();
        let d = ProductId::new();
        let ao = SubscriptionAddOnId::new();
        let lines = vec![
            line(1000, Some(a), None, LineItemKind::Regular), // plan component, product a
            line(500, Some(b), None, LineItemKind::Regular),  // product b (out of scope)
            line(3000, Some(c), None, LineItemKind::Regular), // EXTRA with product c → counts
            line(700, Some(d), Some(ao), LineItemKind::Regular), // add-on, product d (out of scope)
        ];
        // Scope = products {a, c}: plan component a + extra c count; b and the add-on (d) don't.
        assert_eq!(
            counted_total(&lines, &MinimumScope::Products(vec![a, c])),
            4000
        );
    }

    #[test]
    fn counted_total_excludes_shortfall_lines() {
        let a = ProductId::new();
        let lines = vec![
            line(1000, Some(a), None, LineItemKind::Regular),
            line(500, None, None, LineItemKind::MinimumShortfall),
        ];
        assert_eq!(counted_total(&lines, &MinimumScope::AllComponents), 1000);
    }

    /// A true-up of September 2024 with `floor`, `prepaid` up front over the month.
    fn september(floor: i64, prepaid: i64) -> core::TrueUp {
        let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
        core::TrueUp {
            start: day(9, 1),
            end: day(10, 1),
            floor,
            scope: core::Scope::AllComponents,
            cut: None,
            prepaid: vec![(day(9, 1), day(10, 1), prepaid)],
            trued_up: 0,
            carried: vec![],
        }
    }

    fn project(
        floor: i64,
        counted: i64,
        usage: i64,
        prepaid: i64,
        elapsed: f64,
    ) -> Option<ShortfallProjection> {
        project_shortfall(&september(floor, prepaid), counted, usage, elapsed)
    }

    #[test]
    fn projected_shortfall_extrapolates_usage_only_net_of_an_up_front_shortfall() {
        let p = project(50_000, 17_000, 3_000, 0, 0.25).unwrap();
        assert_eq!(p.counted_to_date_cents, 17_000);
        assert_eq!(p.projected_cents, 26_000);
        assert_eq!(p.projected_shortfall_cents, 24_000);

        let done = project(50_000, 17_000, 3_000, 0, 1.0).unwrap();
        assert_eq!(done.projected_cents, 17_000);

        let covered = project(20_000, 17_000, 3_000, 0, 0.5).unwrap();
        assert_eq!(covered.projected_cents, 20_000);
        assert_eq!(covered.projected_shortfall_cents, 0);

        let paid_up_front = project(50_000, 17_000, 3_000, 20_000, 0.25).unwrap();
        assert_eq!(paid_up_front.projected_shortfall_cents, 4_000);

        assert_eq!(project(50_000, 17_000, 3_000, 0, 0.0), None);
        assert_eq!(project(50_000, 17_000, 3_000, 0, f64::NAN), None);
        let third = project(50_000, 17_000, 3_000, 0, 1.0 / 3.0).unwrap();
        assert_eq!(
            third.projected_cents, 23_000,
            "6 000 more usage, rounded once"
        );
    }

    /// The rule before the true-up applied its floor on the opening invoice when it billed an
    /// in-scope fee there (a one-time fee's zero-width line too) or charged a shortfall; beside
    /// usage or out-of-scope fees alone it waited for the renewal.
    #[test]
    fn the_opening_invoice_tells_how_old_shortfalls_were_dated() {
        use common_domain::ids::BillableMetricId;
        let (a, b) = (ProductId::new(), ProductId::new());
        let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
        let dated = |product, metric: Option<BillableMetricId>, start, end| LineItem {
            start_date: start,
            end_date: end,
            metric_id: metric,
            ..line(1_000, Some(product), None, LineItemKind::Regular)
        };
        let all = MinimumScope::AllComponents;
        let fee = dated(a, None, day(9, 1), day(10, 1));
        let one_time = dated(a, None, day(9, 1), day(9, 1));
        let usage = dated(a, Some(BillableMetricId::new()), day(9, 1), day(10, 1));
        let shortfall = LineItem {
            start_date: day(9, 1),
            end_date: day(10, 1),
            ..line(8_000, None, None, LineItemKind::MinimumShortfall)
        };
        use UpFrontShortfall::{Prepaid, TrueUpOfPeriodBefore};
        assert_eq!(
            up_front_shortfall_of(std::slice::from_ref(&fee), &all),
            Prepaid
        );
        assert_eq!(up_front_shortfall_of(&[one_time], &all), Prepaid);
        assert_eq!(up_front_shortfall_of(&[shortfall], &all), Prepaid);
        assert_eq!(up_front_shortfall_of(&[usage], &all), TrueUpOfPeriodBefore);
        assert_eq!(up_front_shortfall_of(&[], &all), TrueUpOfPeriodBefore);
        assert_eq!(
            up_front_shortfall_of(std::slice::from_ref(&fee), &MinimumScope::Products(vec![a])),
            Prepaid
        );
        assert_eq!(
            up_front_shortfall_of(&[fee], &MinimumScope::Products(vec![b])),
            TrueUpOfPeriodBefore
        );
    }

    /// A shortfall dated by the invoice's date is always recomputed; one past it only when the
    /// period's true-up replaces it, the rule before having charged it as that true-up.
    #[test]
    fn an_old_shortfall_is_replaced_only_by_the_true_up_of_the_period_it_measured() {
        let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
        let old = LineItem {
            start_date: day(10, 1),
            end_date: day(11, 1),
            ..line(362, None, None, LineItemKind::MinimumShortfall)
        };
        let true_up = |up_front| PeriodTrueUp {
            start: day(9, 1),
            end: day(10, 1),
            minimum: SubscriptionMinimum {
                subscription_id: SubscriptionId::new(),
                amount_subunit: 756,
                scope: MinimumScope::AllComponents,
                is_override: false,
            },
            true_up: september(756, 0),
            prior: vec![],
            up_front,
            provisional: vec![],
            queue: Queue::default(),
        };
        let before = true_up(Some(UpFrontShortfall::TrueUpOfPeriodBefore));
        let prepaid = true_up(Some(UpFrontShortfall::Prepaid));
        assert!(is_true_up(&old, day(10, 1), Some(&before)));
        assert!(!is_true_up(&old, day(10, 1), Some(&prepaid)));
        assert!(!is_true_up(&old, day(10, 1), None));
        let dated_in = LineItem {
            start_date: day(9, 1),
            end_date: day(10, 1),
            ..old.clone()
        };
        assert!(is_true_up(&dated_in, day(10, 1), None));
        assert!(!is_true_up(
            &line(1, None, None, LineItemKind::Regular),
            day(10, 1),
            Some(&before)
        ));
    }
}
