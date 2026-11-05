//! Minimum commitment as a period true-up.
//!
//! The minimum of a completed period P is trued up on the Recurring invoice that closes P: a
//! renewal bills the period just ended, a termination the part of it served. P's in-scope spend
//! is its arrears on that invoice plus what the subscription's other invoices dated in P billed
//! for P (the advance fees of its opening invoice, immediate increases). Fees count as billed,
//! never net of credit notes; a cancellation counts only the days served. A product-scoped
//! minimum bills one line per scoped product, each taxed with its product (OQ17).

use std::collections::HashMap;

use chrono::NaiveDate;
use nanoid::nanoid;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};

use crate::StoreResult;
use crate::domain::enums::{BillingPeriodEnum, InvoiceType};
use crate::domain::plan_minimums::MinimumScope;
use crate::domain::subscription_minimums::SubscriptionMinimum;
use crate::domain::{ChargeType, LineItem, LineItemKind, SubscriptionDetails};
use crate::errors::StoreError;
use crate::services::subscriptions::proration::DayFraction;
use crate::store::PgConn;
use common_domain::ids::{ProductId, SubscriptionId, TenantId};
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
    /// The minimum prorated to the days served.
    pub floor: i64,
    /// Spend of the period billed on the subscription's other invoices (only the days served when
    /// a cancellation cut it short), by product (`None` for a product-less line).
    pub prior: Vec<(Option<ProductId>, i64)>,
    /// Shortfall the period's opening invoice prepaid up front (the rule before the true-up, kept
    /// by invoices written then), for the days served.
    pub already_billed: i64,
    /// How that rule dated its shortfalls on the subscription, read only when the period or the
    /// invoice closing it carries one.
    pub up_front: Option<UpFrontShortfall>,
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
    /// The minimum over the period: whole for each grid period served in full, by days otherwise.
    pub(crate) fn floor(&self, minimum: i64) -> i64 {
        self.served
            .iter()
            .map(|(days, nominal_days)| {
                if days == nominal_days {
                    minimum
                } else {
                    DayFraction::new(*days, *nominal_days).prorate(minimum)
                }
            })
            .sum()
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
    let windows: Vec<(Period, Period)> = match run {
        Some(run) => run.arrear_windows(invoice_date, cadence.as_months(), anchor, settles_early),
        None => {
            let period =
                calculate_arrear_period_range(invoice_date, billing_start, anchor, cadence);
            let grid = find_period_containing_date(
                billing_start,
                invoice_date.pred_opt()?,
                cadence,
                anchor,
            );
            let nominal_start =
                subtract_months_at_billing_day(grid.end, cadence.as_months(), anchor)
                    .unwrap_or(grid.start);
            vec![(
                period,
                Period {
                    start: nominal_start,
                    end: grid.end,
                },
            )]
        }
    };
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

/// The days of a paid trial, from the billing start (its EndTrial fires after as many days).
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

/// The true-up shortfall: what the period's floor asks above its counted spend and above what an
/// opening invoice already charged for it up front. Never negative: a true-up is never credited.
pub(crate) fn trued_up_shortfall(floor: i64, counted: i64, already_billed: i64) -> i64 {
    (floor - counted - already_billed).max(0)
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

    // Fees count as billed, never net of credit notes (a goodwill note is not clawed back). A
    // cancellation gives the unused days of what spans it back: only the days served count.
    let cut = (sub.status == crate::domain::SubscriptionStatusEnum::Cancelled && period.cut_short)
        .then_some(end);
    let served_share = |amount: i64, from: NaiveDate, to: NaiveDate| match cut {
        Some(cut) if from <= cut && cut < to && amount > 0 => {
            amount - DayFraction::remaining(from, to, cut).prorate(amount)
        }
        _ => amount,
    };
    let mut prior = Vec::new();
    let mut up_front: HashMap<(NaiveDate, NaiveDate), i64> = HashMap::new();
    let rows = diesel_models::invoices::InvoiceRow::list_period_spend_invoices(
        conn,
        sub.tenant_id,
        sub.id,
        start,
        end,
        invoice.map(|invoice| invoice.id),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    for row in rows {
        let invoice: crate::domain::Invoice = row.try_into()?;
        // A correction re-bills lines its parent's note credited: the parent's lines count.
        if invoice.parent_invoice_id.is_some() {
            continue;
        }
        let mut lines = invoice.line_items.clone();
        if invoice.subscription_id.is_none() {
            let owners =
                crate::services::credit::scope::LineOwners::load_tx(conn, sub.tenant_id, &lines)
                    .await?;
            lines.retain(|line| {
                crate::services::credit::scope::line_subscription(line, None, &owners)
                    == Some(sub.id)
            });
        }
        let quoted = |line: &LineItem| {
            if invoice.tax_inclusive {
                line.clone().into_tax_inclusive_quote().amount_subtotal
            } else {
                line.amount_subtotal
            }
        };
        for line in lines
            .iter()
            .filter(|line| line.start_date >= start && line.start_date < end)
        {
            match line.kind {
                LineItemKind::MinimumShortfall
                    if is_up_front(line, invoice.billing_period_start) =>
                {
                    *up_front
                        .entry((line.start_date, line.end_date))
                        .or_insert(0) += quoted(line);
                }
                LineItemKind::Regular => {
                    let amount = if line.metric_id.is_none() {
                        served_share(quoted(line), line.start_date, line.end_date)
                    } else {
                        quoted(line)
                    };
                    prior.push((line.product_id, amount));
                }
                _ => {}
            }
        }
    }
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
    // An up-front shortfall is split over products like the true-up: its days are cut once.
    let already_billed = match up_front_rule {
        Some(UpFrontShortfall::Prepaid) => up_front
            .into_iter()
            .map(|((from, to), billed)| served_share(billed, from, to))
            .sum(),
        _ => 0,
    };

    Ok(Some(PeriodTrueUp {
        start,
        end,
        minimum,
        floor,
        prior,
        already_billed,
        up_front: up_front_rule,
    }))
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

/// Appends the true-up of the period the invoice closes when its in-scope spend falls short of
/// the prorated minimum. An earlier true-up line is dropped first, so a refresh recomputes it.
pub fn append_minimum_shortfall_if_any(
    invoice_type: &InvoiceType,
    subscription_details: &SubscriptionDetails,
    lines: &mut Vec<LineItem>,
    invoice_date: NaiveDate,
    true_up: Option<&PeriodTrueUp>,
) -> StoreResult<()> {
    if !matches!(invoice_type, InvoiceType::Recurring) {
        return Ok(());
    }
    lines.retain(|l| !is_true_up(l, invoice_date, true_up));
    let Some(true_up) = true_up else {
        return Ok(());
    };
    let scope = &true_up.minimum.scope;
    let weights = counted_spend(lines, true_up);
    let counted: i64 = weights.iter().map(|(_, amount)| amount).sum();
    let shortfall = trued_up_shortfall(true_up.floor, counted, true_up.already_billed);
    if shortfall == 0 {
        return Ok(());
    }

    let sub = &subscription_details.subscription;
    // For a product-scoped minimum, name the covered product(s) on the line so it's clear which
    // commitment is being trued up (e.g. "Minimum commitment (Bandwidth)").
    let name = minimum_line_name(subscription_details, scope);
    let weights: Vec<(ProductId, i64)> = weights
        .into_iter()
        .filter_map(|(product, amount)| product.map(|product| (product, amount)))
        .collect();
    let split = split_shortfall(
        scope,
        &weights,
        &carried_scope_products(subscription_details, scope),
        shortfall,
    );
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

/// The period's in-scope spend by product: its lines on this invoice, then on the others.
fn counted_spend(lines: &[LineItem], true_up: &PeriodTrueUp) -> Vec<(Option<ProductId>, i64)> {
    let scope = &true_up.minimum.scope;
    counted_lines(lines, scope)
        .filter(|line| line.start_date >= true_up.start && line.start_date < true_up.end)
        .map(|line| (line.product_id, line.amount_subtotal))
        .chain(
            true_up
                .prior
                .iter()
                .copied()
                .filter(|(product, _)| in_scope(scope, *product)),
        )
        .collect()
}

fn in_scope(scope: &MinimumScope, product_id: Option<ProductId>) -> bool {
    match scope {
        MinimumScope::AllComponents => true,
        MinimumScope::Products(_) => product_id.is_some_and(|id| scope.includes(id)),
    }
}

fn shortfall_line(
    name: String,
    amount: i64,
    product_id: Option<ProductId>,
    start_date: NaiveDate,
    end_date: NaiveDate,
    subscription_id: SubscriptionId,
) -> LineItem {
    LineItem {
        local_id: nanoid!(),
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
    }
}

/// How a shortfall is billed: one product-less line for an all-components minimum (the default
/// tax); for a product-scoped one, a line per scoped product pro rata to what it counted toward
/// the floor (largest remainder), equally over the scoped products when none counted anything.
pub(crate) fn split_shortfall(
    scope: &MinimumScope,
    weights: &[(ProductId, i64)],
    carried: &[ProductId],
    shortfall: i64,
) -> Vec<(Option<ProductId>, i64)> {
    let MinimumScope::Products(ids) = scope else {
        return vec![(None, shortfall)];
    };
    let mut by_product: Vec<(ProductId, i64)> = Vec::new();
    for (product, amount) in weights.iter().filter(|(p, _)| ids.contains(p)) {
        match by_product.iter_mut().find(|(p, _)| p == product) {
            Some(entry) => entry.1 += amount,
            None => by_product.push((*product, *amount)),
        }
    }
    by_product.retain(|(_, amount)| *amount > 0);
    if by_product.is_empty() {
        let pool: Vec<ProductId> = if carried.is_empty() {
            ids.clone()
        } else {
            carried.to_vec()
        };
        by_product = pool.into_iter().map(|product| (product, 1)).collect();
    }
    if by_product.is_empty() {
        return vec![(None, shortfall)];
    }
    let total: i64 = by_product.iter().map(|(_, weight)| weight).sum();
    let exact: Vec<Decimal> = by_product
        .iter()
        .map(|(_, weight)| Decimal::from(shortfall) * Decimal::from(*weight) / Decimal::from(total))
        .collect();
    by_product
        .iter()
        .zip(meteroid_tax::largest_remainder(&exact))
        .filter(|(_, amount)| *amount != 0)
        .map(|((product, _), amount)| (Some(*product), amount))
        .collect()
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

/// The in-scope revenue lines, counted by `amount_subtotal` (GROSS, pre-discount).
///
/// The floor is measured against **gross list spend**, before any coupon/discount is applied.
/// This is the standard B2B "minimum committed spend" semantics: a discount is a
/// negotiated benefit on actual consumption, not something the floor claws back. Consequences:
/// - a discount that takes the customer *below* the floor still benefits them (the top-up is
///   measured against gross spend, not the discounted amount);
/// - a 100%-off coupon on a product that meets the floor produces **no** shortfall, instead of
///   silently re-billing the entire minimum.
///
/// `amount_subtotal` is unaffected by `distribute_discount` (which only mutates `taxable_amount`),
/// so this is independent of where the shortfall is appended relative to discount distribution.
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
    let counted: i64 = counted_spend(lines, true_up)
        .iter()
        .map(|(_, amount)| amount)
        .sum();
    let usage: i64 = counted_lines(lines, scope)
        .filter(|l| l.start_date >= true_up.start && l.start_date < true_up.end)
        .filter(|l| extrapolates(invoice_details, l))
        .map(|l| l.amount_subtotal)
        .sum();
    project_shortfall(
        true_up.floor,
        counted,
        usage,
        true_up.already_billed,
        elapsed,
    )
    .map(|p| ShortfallProjection {
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
    floor: i64,
    counted: i64,
    usage: i64,
    already_billed: i64,
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
        floor_cents: floor,
        counted_to_date_cents: counted,
        projected_cents: projected,
        projected_shortfall_cents: trued_up_shortfall(floor, projected, already_billed),
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

    #[test]
    fn projected_shortfall_extrapolates_usage_only_net_of_an_up_front_shortfall() {
        let p = project_shortfall(50_000, 17_000, 3_000, 0, 0.25).unwrap();
        assert_eq!(p.counted_to_date_cents, 17_000);
        assert_eq!(p.projected_cents, 26_000);
        assert_eq!(p.projected_shortfall_cents, 24_000);

        let done = project_shortfall(50_000, 17_000, 3_000, 0, 1.0).unwrap();
        assert_eq!(done.projected_cents, 17_000);

        let covered = project_shortfall(20_000, 17_000, 3_000, 0, 0.5).unwrap();
        assert_eq!(covered.projected_cents, 20_000);
        assert_eq!(covered.projected_shortfall_cents, 0);

        let paid_up_front = project_shortfall(50_000, 17_000, 3_000, 20_000, 0.25).unwrap();
        assert_eq!(paid_up_front.projected_shortfall_cents, 4_000);

        assert_eq!(project_shortfall(50_000, 17_000, 3_000, 0, 0.0), None);
        assert_eq!(project_shortfall(50_000, 17_000, 3_000, 0, f64::NAN), None);
        let third = project_shortfall(50_000, 17_000, 3_000, 0, 1.0 / 3.0).unwrap();
        assert_eq!(
            third.projected_cents, 23_000,
            "6 000 more usage, rounded once"
        );
    }

    /// Fee 29.00 in advance, minimum 1 000.00: September's usage of 0 leaves 971.00 to true up.
    /// An opening invoice that charged 971.00 up front under the earlier rule leaves nothing, or
    /// what a note gave back of it; a true-up is never negative.
    #[test]
    fn the_true_up_nets_what_the_opening_invoice_charged_up_front() {
        assert_eq!(trued_up_shortfall(100_000, 2_900, 0), 97_100);
        assert_eq!(trued_up_shortfall(100_000, 2_900, 97_100), 0);
        assert_eq!(trued_up_shortfall(100_000, 2_900, 97_100 - 68_910), 68_910);
        assert_eq!(trued_up_shortfall(29_032, 842 + 90_000, 0), 0);
        assert_eq!(trued_up_shortfall(29_032, 842, 97_100), 0);
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
            floor: 756,
            prior: vec![],
            already_billed: 0,
            up_front,
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

    /// P's spend is this invoice's lines dated in P (its arrears, not the next period's advance)
    /// plus what the period's other invoices billed, both within the minimum's scope.
    #[test]
    fn counted_spend_takes_the_period_lines_of_this_invoice_and_the_others() {
        let (a, b) = (ProductId::new(), ProductId::new());
        let dated = |amount, product, start: NaiveDate, end: NaiveDate| LineItem {
            start_date: start,
            end_date: end,
            ..line(amount, Some(product), None, LineItemKind::Regular)
        };
        let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        let lines = vec![
            dated(90_000, a, day(9, 1), day(10, 1)),
            dated(2_900, a, day(10, 1), day(11, 1)),
            dated(500, b, day(9, 1), day(10, 1)),
            LineItem {
                start_date: day(9, 1),
                ..line(7_000, None, None, LineItemKind::MinimumShortfall)
            },
        ];
        let true_up = |scope| PeriodTrueUp {
            start: day(9, 1),
            end: day(10, 1),
            minimum: SubscriptionMinimum {
                subscription_id: SubscriptionId::new(),
                amount_subunit: 100_000,
                scope,
                is_override: false,
            },
            floor: 100_000,
            prior: vec![(Some(a), 2_900), (Some(b), 1_000), (None, 300)],
            already_billed: 0,
            up_front: None,
        };
        let sum = |spend: Vec<(Option<ProductId>, i64)>| spend.iter().map(|(_, v)| v).sum::<i64>();
        assert_eq!(
            sum(counted_spend(&lines, &true_up(MinimumScope::AllComponents))),
            90_000 + 500 + 2_900 + 1_000 + 300
        );
        assert_eq!(
            sum(counted_spend(
                &lines,
                &true_up(MinimumScope::Products(vec![a]))
            )),
            90_000 + 2_900
        );
    }
}
