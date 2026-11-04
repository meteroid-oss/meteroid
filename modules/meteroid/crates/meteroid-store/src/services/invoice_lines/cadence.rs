//! A plan switch can change a subscription's cadence (monthly to quarterly, …). Periods after it
//! run on a grid of the new cadence from the first renewal on it, detected from the components
//! the switch closed and opened, so subscriptions switched by any release are read alike. Where
//! an earlier release left that grid is decided once and recorded on the switch's event.

use chrono::NaiveDate;
use error_stack::Report;

use crate::StoreResult;
use crate::domain::SubscriptionDetails;
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::utils::periods::CadenceRun;
use common_domain::ids::{SubscriptionId, SubscriptionPriceComponentId, TenantId};
use diesel_models::invoices::InvoiceRow;
use diesel_models::lock::CustomerLock;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::SubscriptionRow;

/// The subscription a cadence is read for, as it stands when the invoice is computed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CadenceSubject {
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub billing_start: Option<NaiveDate>,
    pub billing_day: u32,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
}

impl CadenceSubject {
    pub(crate) fn of_row(sub: &SubscriptionRow) -> Self {
        Self {
            tenant_id: sub.tenant_id,
            subscription_id: sub.id,
            billing_start: sub.billing_start_date,
            billing_day: sub.billing_day_anchor as u32,
            current_period_start: sub.current_period_start,
            current_period_end: sub.current_period_end,
        }
    }

    pub(crate) fn of(details: &SubscriptionDetails) -> Self {
        let sub = &details.subscription;
        Self {
            tenant_id: sub.tenant_id,
            subscription_id: sub.id,
            billing_start: sub.billing_start_date,
            billing_day: u32::from(sub.billing_day_anchor),
            current_period_start: sub.current_period_start,
            current_period_end: sub.current_period_end,
        }
    }
}

/// The cadence runs up to `invoice_date` (before it, with `switch_on_date` false), latest first:
/// one per plan switch that changed the cadence. Empty: the billing start's grid applies.
pub(crate) async fn load_cadence_runs(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    invoice_date: NaiveDate,
    switch_on_date: bool,
) -> StoreResult<Vec<CadenceRun>> {
    cadence_runs_of(
        conn,
        CadenceSubject::of(details),
        invoice_date,
        switch_on_date,
    )
    .await
}

/// The latest of [`load_cadence_runs`]: the grid `invoice_date` falls on.
pub(crate) async fn load_cadence_run(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    invoice_date: NaiveDate,
    switch_on_date: bool,
) -> StoreResult<Option<CadenceRun>> {
    Ok(
        load_cadence_runs(conn, details, invoice_date, switch_on_date)
            .await?
            .into_iter()
            .next(),
    )
}

pub(crate) async fn cadence_runs_of(
    conn: &mut PgConn,
    subject: CadenceSubject,
    invoice_date: NaiveDate,
    switch_on_date: bool,
) -> StoreResult<Vec<CadenceRun>> {
    Ok(decide_runs(conn, subject, invoice_date, switch_on_date)
        .await?
        .into_iter()
        .map(|decided| decided.run)
        .collect())
}

/// Records, once, the grid each change of cadence runs on, read from the subscription as it stands
/// (before a renewal, a termination or a refresh changes it). An ended subscription records
/// nothing: an instance of the previous release may have ended it, leaving its period unreadable.
pub(crate) async fn record_cadence_origins_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    subject: CadenceSubject,
    invoice_date: NaiveDate,
) -> StoreResult<()> {
    if subject.current_period_end.is_none() {
        return Ok(());
    }
    for decided in decide_runs(conn, subject, invoice_date, true).await? {
        if let Some(origin) = decided.undecided {
            SubscriptionEventRow::record_cadence_origin(
                conn,
                lock,
                subject.subscription_id,
                decided.run.switched_on,
                origin,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        }
    }
    Ok(())
}

struct DecidedRun {
    run: CadenceRun,
    /// The origin read now from what the earlier release left, not recorded yet.
    undecided: Option<NaiveDate>,
}

async fn decide_runs(
    conn: &mut PgConn,
    subject: CadenceSubject,
    invoice_date: NaiveDate,
    switch_on_date: bool,
) -> StoreResult<Vec<DecidedRun>> {
    let up_to = if switch_on_date {
        invoice_date
    } else {
        match invoice_date.pred_opt() {
            Some(day) => day,
            None => return Ok(vec![]),
        }
    };
    let switches =
        SubscriptionEventRow::list_plan_switch_dates(conn, subject.subscription_id, up_to)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let mut runs = Vec::new();
    let mut later_switch: Option<NaiveDate> = None;
    for switched_on in switches {
        let Some(months) = new_cadence(conn, subject.subscription_id, switched_on).await? else {
            continue;
        };
        let (starts, previous) = InvoiceRow::recurring_period_starts_around(
            conn,
            subject.tenant_id,
            subject.subscription_id,
            switched_on,
            invoice_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let mut run = CadenceRun {
            switched_on,
            origin: match starts.first() {
                Some(first) => (*first).min(invoice_date.max(switched_on)),
                None => invoice_date.max(switched_on),
            },
            previous_renewal: previous.or(subject.billing_start).unwrap_or(switched_on),
        };
        let recorded =
            SubscriptionEventRow::cadence_origin(conn, subject.subscription_id, switched_on)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let mut undecided = None;
        match recorded {
            // Recorded for a later invoice: an earlier one still runs from the switch's grid.
            Some(origin) => {
                if origin <= invoice_date {
                    run.origin = origin;
                }
            }
            None => {
                if let Some(&renewal) = starts.get(1)
                    && renewal > run.origin
                    && renewal <= invoice_date
                    && later_switch.is_none_or(|later| renewal < later)
                    && !run.is_boundary(renewal, months, subject.billing_day)
                {
                    let latest = later_switch.is_none();
                    if renewed_a_whole_period(conn, &subject, renewal, months, latest).await? {
                        run.origin = renewal;
                    }
                    undecided = Some(run.origin);
                }
            }
        }
        runs.push(DecidedRun { run, undecided });
        later_switch = Some(switched_on);
    }
    Ok(runs)
}

/// Whether the renewal on `renewal`, off the switch's grid, was one an earlier release made for a
/// whole period of the new cadence, renewing on a grid from it. A renewal this release realigned
/// bills nothing of that cadence; only the latest change's grid can be read from the period.
async fn renewed_a_whole_period(
    conn: &mut PgConn,
    subject: &CadenceSubject,
    renewal: NaiveDate,
    months: u32,
    latest: bool,
) -> StoreResult<bool> {
    let own_grid = CadenceRun {
        switched_on: renewal,
        origin: renewal,
        previous_renewal: renewal,
    };
    let current = CadenceRun {
        origin: subject.current_period_start,
        ..own_grid
    };
    if latest
        && own_grid.is_boundary(subject.current_period_start, months, subject.billing_day)
        && subject.current_period_end == current.boundary(1, months, subject.billing_day)
    {
        return Ok(true);
    }
    bills_new_cadence_on(conn, subject, renewal, months).await
}

/// Whether the recurring invoice for `on` bills a component of the `months` cadence live then.
async fn bills_new_cadence_on(
    conn: &mut PgConn,
    subject: &CadenceSubject,
    on: NaiveDate,
    months: u32,
) -> StoreResult<bool> {
    #[derive(serde::Deserialize)]
    struct BilledComponent {
        #[serde(default)]
        sub_component_id: Option<SubscriptionPriceComponentId>,
    }
    let mut components = Vec::new();
    for row in SubscriptionComponentRow::list_component_history_for_period(
        conn,
        &subject.subscription_id,
        on,
        on.succ_opt().unwrap_or(on),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    {
        let component: crate::domain::SubscriptionComponent = row.try_into()?;
        if component
            .period
            .as_billing_period_opt()
            .is_some_and(|period| period.as_months() == months)
        {
            components.push(component.id);
        }
    }
    if components.is_empty() {
        return Ok(false);
    }
    for lines in
        InvoiceRow::recurring_line_items_on(conn, subject.tenant_id, subject.subscription_id, on)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
    {
        let lines: Vec<BilledComponent> = serde_json::from_value(lines).map_err(|error| {
            Report::new(StoreError::SerdeError(
                "Failed to read a renewal's lines".to_string(),
                error,
            ))
        })?;
        if lines.iter().any(|line| {
            line.sub_component_id
                .is_some_and(|id| components.contains(&id))
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The shortest cadence of the components the switch on `on` opened, in months, when it differs
/// from the shortest of those it closed.
async fn new_cadence(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    on: NaiveDate,
) -> StoreResult<Option<u32>> {
    let rows = SubscriptionComponentRow::list_component_history_for_period(
        conn,
        &subscription_id,
        on.pred_opt().unwrap_or(on),
        on.succ_opt().unwrap_or(on),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let (mut closed, mut opened): (Option<u32>, Option<u32>) = (None, None);
    for row in rows {
        let (is_closed, is_opened) = (row.effective_to == Some(on), row.effective_from == on);
        if !is_closed && !is_opened {
            continue;
        }
        let component: crate::domain::SubscriptionComponent = row.try_into()?;
        let Some(months) = component
            .period
            .as_billing_period_opt()
            .map(|period| period.as_months())
        else {
            continue;
        };
        let shortest = |current: Option<u32>| Some(current.map_or(months, |c| c.min(months)));
        if is_closed {
            closed = shortest(closed);
        }
        if is_opened {
            opened = shortest(opened);
        }
    }
    Ok(match (closed, opened) {
        (Some(before), Some(after)) if before != after => Some(after),
        _ => None,
    })
}
