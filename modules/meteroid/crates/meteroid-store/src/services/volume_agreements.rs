//! Volume agreements in billing (ADR-0021): the volume a member's pooled fees pick their tiers at.
//! Billing reads the agreements whatever the gate; the module manages them.

use std::collections::{BTreeMap, BTreeSet};

use billing_core::evaluate::usage_windows;
use billing_core::pricer::{UsageRead, UsageReads};
use billing_core::timeline::Grid;
use chrono::NaiveDate;
use common_domain::ids::{BillableMetricId, SubscriptionId, TenantId, VolumeAgreementId};
use diesel_models::PgConn;
use diesel_models::volume_agreements::{
    VolumeAgreementBasisRow, VolumeAgreementMemberRow, VolumeAgreementMetricRow, VolumeAgreementRow,
};
use error_stack::Report;
use rust_decimal::Decimal;

use crate::StoreResult;
use crate::domain::enums::BillingPeriodEnum;
use crate::errors::StoreError;
use crate::services::clients::usage::UsageClient;
use crate::services::evaluate::bill::midnight;
use crate::services::evaluate::load::LoadedFacts;
use crate::services::evaluate::pricer::{StorePricer, billed_total};
use crate::services::evaluate::usage::read_one;

/// An agreement window `[start, end)`.
pub type Window = (NaiveDate, NaiveDate);

#[derive(Clone, Debug)]
pub struct Membership {
    pub agreement: VolumeAgreementRow,
    pub metrics: Vec<BillableMetricId>,
    pub from: NaiveDate,
    pub until: Option<NaiveDate>,
}

impl Membership {
    /// The agreement window holding `day`, when the membership and the agreement cover it whole.
    pub fn window_of(&self, day: NaiveDate) -> Option<Window> {
        let window = agreement_grid(&self.agreement)?.period_containing(day)?;
        let covered = window.0 >= self.from.max(self.agreement.start_date)
            && self.until.is_none_or(|until| window.1 <= until)
            && self.agreement.end_date.is_none_or(|end| window.1 <= end);
        covered.then_some(window)
    }
}

/// The grid of the agreement's windows, from its start on its anchor.
pub fn agreement_grid(agreement: &VolumeAgreementRow) -> Option<Grid> {
    let months = BillingPeriodEnum::from(agreement.cadence.clone()).as_months();
    let anchor = u32::try_from(agreement.billing_day_anchor).ok()?;
    Grid::from_origin(agreement.start_date, anchor, months)
}

/// Another member of one of the subscription's agreements, with its facts.
#[derive(Clone, Debug)]
pub struct Peer {
    pub agreement_id: VolumeAgreementId,
    pub subscription_id: SubscriptionId,
    pub from: NaiveDate,
    pub until: Option<NaiveDate>,
    pub loaded: LoadedFacts,
}

/// What pricing a member reads of its agreements: none for a subscription in none.
#[derive(Clone, Debug)]
pub struct PoolContext {
    pub memberships: Vec<Membership>,
    pub peers: Vec<Peer>,
    pub closed: Vec<VolumeAgreementBasisRow>,
}

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::<Report<StoreError>>::into)
}

/// The agreements `subscription_id` is a member of, their other members' facts and their closed
/// windows.
pub async fn load_pool(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    as_of: NaiveDate,
) -> StoreResult<Option<PoolContext>> {
    let rows =
        db(VolumeAgreementMemberRow::memberships_of(conn, tenant_id, subscription_id).await)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let ids: Vec<VolumeAgreementId> = rows.iter().map(|(_, a)| a.id).collect();
    let metrics = db(VolumeAgreementMetricRow::list_of(conn, &ids).await)?;
    let memberships = rows
        .into_iter()
        .map(|(member, agreement)| Membership {
            metrics: metrics
                .iter()
                .filter(|m| m.agreement_id == agreement.id)
                .map(|m| m.billable_metric_id)
                .collect(),
            from: member.from_date,
            until: member.until_date,
            agreement,
        })
        .collect::<Vec<_>>();

    let mut peers = Vec::new();
    let mut closed = Vec::new();
    for agreement_id in ids.iter().collect::<BTreeSet<_>>() {
        closed.extend(db(VolumeAgreementBasisRow::of_agreement(
            conn,
            *agreement_id,
        )
        .await)?);
        for member in db(VolumeAgreementMemberRow::list_of(conn, tenant_id, *agreement_id).await)? {
            if member.subscription_id == subscription_id {
                continue;
            }
            let loaded = Box::pin(crate::services::evaluate::load::load_facts_unpooled(
                conn,
                tenant_id,
                member.subscription_id,
                as_of,
            ))
            .await?;
            peers.push(Peer {
                agreement_id: *agreement_id,
                subscription_id: member.subscription_id,
                from: member.from_date,
                until: member.until_date,
                loaded,
            });
        }
    }
    Ok(Some(PoolContext {
        memberships,
        peers,
        closed,
    }))
}

/// What a subscription's own invoices bill of `metric` in `window`, never below zero: each usage
/// read its evaluation plans within the window once, in what its fee bills it in (ADR-0021 §2).
pub async fn member_usage(
    client: &dyn UsageClient,
    loaded: &LoadedFacts,
    metric: BillableMetricId,
    window: Window,
) -> StoreResult<Decimal> {
    let mut facts = loaded.facts.clone();
    facts.reconcile_from = None;
    facts.arrears_from = None;
    let until = midnight(window.1);
    let begun = usage_windows(&facts, until, &StorePricer)
        .map_err(crate::services::invoice_lines::fees::pricing_error)?;
    let mut reads = BTreeMap::new();
    for billed in begun {
        let inside = billed.read.from >= midnight(window.0) && billed.read.until <= until;
        if inside && billed.row.fee.metric() == Some(metric) {
            reads
                .entry(UsageRead {
                    metric,
                    window: billed.read,
                })
                .or_insert(&billed.row.fee);
        }
    }
    let mut total = Decimal::ZERO;
    for (read, fee) in reads {
        total += billed_total(fee, &read_one(client, loaded, &read).await?);
    }
    Ok(total.max(Decimal::ZERO))
}

/// The bases of the pooled reads of `reads`, written into them: a closed window's, else the live
/// sum of every member's usage over it.
pub async fn serve_bases(
    client: &dyn UsageClient,
    loaded: &LoadedFacts,
    reads: &mut UsageReads,
    needed: &[UsageRead],
) -> StoreResult<()> {
    let Some(pool) = loaded.pool.as_deref() else {
        return Ok(());
    };
    let mut served: BTreeMap<(VolumeAgreementId, BillableMetricId, Window), Decimal> =
        BTreeMap::new();
    for read in needed {
        let Some((membership, window)) = pool.memberships.iter().find_map(|m| {
            let window = m.window_of(read.window.from.date_naive())?;
            (m.metrics.contains(&read.metric) && read.window.until <= midnight(window.1))
                .then_some((m, window))
        }) else {
            continue;
        };
        let key = (membership.agreement.id, read.metric, window);
        if let Some(basis) = served.get(&key) {
            reads.insert_basis(*read, *basis);
            continue;
        }
        let basis = match pool.closed.iter().find(|row| {
            row.agreement_id == key.0
                && row.billable_metric_id == key.1
                && row.period_start == window.0
        }) {
            Some(row) => row.quantity,
            None => live_basis(client, loaded, pool, key.0, read.metric, window).await?,
        };
        served.insert(key, basis);
        reads.insert_basis(*read, basis);
    }
    Ok(())
}

/// Every member's usage of `metric` over `window`, summed (ADR-0021 §2).
pub async fn live_basis(
    client: &dyn UsageClient,
    loaded: &LoadedFacts,
    pool: &PoolContext,
    agreement_id: VolumeAgreementId,
    metric: BillableMetricId,
    window: Window,
) -> StoreResult<Decimal> {
    let mut basis = member_usage(client, loaded, metric, window).await?;
    for peer in pool.peers.iter().filter(|p| {
        p.agreement_id == agreement_id
            && p.from <= window.0
            && p.until.is_none_or(|until| window.1 <= until)
    }) {
        basis += member_usage(client, &peer.loaded, metric, window).await?;
    }
    Ok(basis)
}

/// The windows of the subscription's agreements its recurring invoice dated `invoice_date` bills
/// the usage of: those ending on that day.
fn windows_ending(pool: &PoolContext, invoice_date: NaiveDate) -> Vec<(&Membership, Window)> {
    pool.memberships
        .iter()
        .filter(|m| !m.metrics.is_empty())
        .filter_map(|m| {
            let window = m.window_of(invoice_date.pred_opt()?)?;
            (window.1 == invoice_date).then_some((m, window))
        })
        .collect()
}

/// When a window closes: at its end plus the longest grace of its members' entities (ADR-0021 §4).
pub async fn close_at(
    conn: &mut PgConn,
    tenant_id: TenantId,
    agreement_id: VolumeAgreementId,
    window: Window,
) -> StoreResult<chrono::NaiveDateTime> {
    let grace = db(VolumeAgreementMemberRow::longest_grace_hours(
        conn,
        tenant_id,
        agreement_id,
        window.0,
        window.1,
    )
    .await)?
    .unwrap_or(0)
    .max(0);
    Ok(midnight(window.1).naive_utc() + chrono::Duration::hours(i64::from(grace)))
}

/// The earliest a member's recurring invoice dated `invoice_date` may be finalized: once every
/// window it bills has closed. None for a subscription in no agreement.
pub async fn finalization_hold(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    invoice_date: NaiveDate,
) -> StoreResult<Option<chrono::NaiveDateTime>> {
    let Some(pool) = load_memberships(conn, tenant_id, subscription_id).await? else {
        return Ok(None);
    };
    let mut hold = None;
    for (membership, window) in windows_ending(&pool, invoice_date) {
        let at = close_at(conn, tenant_id, membership.agreement.id, window).await?;
        hold = hold.max(Some(at));
    }
    Ok(hold)
}

/// The subscription's memberships alone, without its peers' facts.
async fn load_memberships(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<Option<PoolContext>> {
    let rows =
        db(VolumeAgreementMemberRow::memberships_of(conn, tenant_id, subscription_id).await)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let ids: Vec<VolumeAgreementId> = rows.iter().map(|(_, a)| a.id).collect();
    let metrics = db(VolumeAgreementMetricRow::list_of(conn, &ids).await)?;
    Ok(Some(PoolContext {
        memberships: rows
            .into_iter()
            .map(|(member, agreement)| Membership {
                metrics: metrics
                    .iter()
                    .filter(|m| m.agreement_id == agreement.id)
                    .map(|m| m.billable_metric_id)
                    .collect(),
                from: member.from_date,
                until: member.until_date,
                agreement,
            })
            .collect(),
        peers: Vec::new(),
        closed: Vec::new(),
    }))
}

/// Closes the windows a member's recurring invoice dated `invoice_date` bills, under each window's
/// lock: every pooled metric's basis written once, from every member's usage. A basis another
/// member's finalization wrote first that differs from this pass's is an error, retried with it.
pub async fn close_windows_tx(
    conn: &mut PgConn,
    client: &dyn UsageClient,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    invoice_date: NaiveDate,
) -> StoreResult<()> {
    let Some(pool) = load_memberships(conn, tenant_id, subscription_id).await? else {
        return Ok(());
    };
    let now = common_clock::now();
    for (membership, window) in windows_ending(&pool, invoice_date) {
        let agreement = &membership.agreement;
        db(VolumeAgreementBasisRow::lock_window(conn, agreement.id, window.0).await)?;
        let closed = db(VolumeAgreementBasisRow::of_window(conn, agreement.id, window.0).await)?;
        let mut rows = Vec::new();
        for (metric, basis) in window_bases(
            conn,
            client,
            tenant_id,
            agreement.id,
            &membership.metrics,
            window,
        )
        .await?
        {
            let metric = &metric;
            match closed.iter().find(|row| row.billable_metric_id == *metric) {
                Some(row) if row.quantity != basis => {
                    return Err(Report::new(StoreError::InvoiceComputationError).attach(format!(
                        "volume agreement {} closed its window from {} at {}, not {basis}: priced again",
                        agreement.id, window.0, row.quantity
                    )));
                }
                Some(_) => {}
                None => rows.push(VolumeAgreementBasisRow {
                    tenant_id,
                    agreement_id: agreement.id,
                    billable_metric_id: *metric,
                    period_start: window.0,
                    period_end: window.1,
                    quantity: basis,
                    closed_at: now,
                }),
            }
        }
        db(VolumeAgreementBasisRow::insert_new(conn, &rows).await)?;
    }
    Ok(())
}

/// Refuses `subscription_id` as a member of `agreement` over `[from, until)` when its pooled fees
/// would not bill the agreement's windows, its currency is another, it is in another agreement
/// then, or another member reads the same customer-level usage (ADR-0021 §1–§3).
pub async fn check_member(
    conn: &mut PgConn,
    tenant_id: TenantId,
    agreement: &VolumeAgreementRow,
    metrics: &[BillableMetricId],
    subscription_id: SubscriptionId,
    from: NaiveDate,
    until: Option<NaiveDate>,
) -> StoreResult<()> {
    let refuse = |msg: String| Err(Report::new(StoreError::InvalidArgument(msg)));
    let grid = agreement_grid(agreement)
        .ok_or_else(|| Report::new(StoreError::InvalidArgument("Invalid agreement grid".into())))?;
    if !grid.is_boundary(from) || until.is_some_and(|until| !grid.is_boundary(until)) {
        return refuse("A membership starts and ends on the agreement's period boundaries".into());
    }
    let oldest_open = db(VolumeAgreementBasisRow::of_agreement(conn, agreement.id).await)?
        .iter()
        .map(|row| row.period_end)
        .max();
    if oldest_open.is_some_and(|open| from < open || until.is_some_and(|until| until < open)) {
        return refuse("A membership cannot change a period the agreement has closed".into());
    }

    let loaded = Box::pin(crate::services::evaluate::load::load_facts_unpooled(
        conn,
        tenant_id,
        subscription_id,
        from,
    ))
    .await?;
    let subscription = db(
        diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
            conn,
            &tenant_id,
            subscription_id,
        )
        .await,
    )?;
    if subscription.subscription.currency != agreement.currency {
        return refuse(format!(
            "The subscription bills in {}, the agreement in {}",
            subscription.subscription.currency, agreement.currency
        ));
    }

    for (member, other) in
        db(VolumeAgreementMemberRow::memberships_of(conn, tenant_id, subscription_id).await)?
    {
        let overlaps = member.from_date < until.unwrap_or(NaiveDate::MAX)
            && member.until_date.is_none_or(|end| from < end);
        if overlaps {
            return refuse(format!(
                "The subscription is in the volume agreement {} over these dates",
                other.name
            ));
        }
    }

    check_windows(&loaded, &grid, metrics, from, until)?;

    let customer_wide = |loaded: &LoadedFacts| {
        loaded
            .metrics
            .iter()
            .any(|m| metrics.contains(&m.id) && m.usage_scope.is_customer_wide())
    };
    if customer_wide(&loaded) {
        for peer in db(VolumeAgreementMemberRow::list_of(conn, tenant_id, agreement.id).await)? {
            let overlaps = peer.from_date < until.unwrap_or(NaiveDate::MAX)
                && peer.until_date.is_none_or(|end| from < end);
            if !overlaps || peer.subscription_id == subscription_id {
                continue;
            }
            let other = Box::pin(crate::services::evaluate::load::load_facts_unpooled(
                conn,
                tenant_id,
                peer.subscription_id,
                from,
            ))
            .await?;
            if other.customer_id == loaded.customer_id && customer_wide(&other) {
                return refuse(
                    "Another member reads the same customer's usage of a pooled metric".into(),
                );
            }
        }
    }
    Ok(())
}

/// Every usage window of a pooled fee within `[from, until)` lies inside one agreement window.
fn check_windows(
    loaded: &LoadedFacts,
    grid: &Grid,
    metrics: &[BillableMetricId],
    from: NaiveDate,
    until: Option<NaiveDate>,
) -> StoreResult<()> {
    let mut facts = loaded.facts.clone();
    facts.reconcile_from = None;
    facts.arrears_from = None;
    let horizon = until.unwrap_or_else(|| {
        from.checked_add_months(chrono::Months::new(grid.months() * 13))
            .unwrap_or(from)
    });
    let begun = usage_windows(&facts, midnight(horizon), &StorePricer)
        .map_err(crate::services::invoice_lines::fees::pricing_error)?;
    for billed in begun {
        let Some(metric) = billed.row.fee.metric() else {
            continue;
        };
        let day = billed.read.from.date_naive();
        if !metrics.contains(&metric) || day < from || until.is_some_and(|until| day >= until) {
            continue;
        }
        let inside = grid
            .period_containing(day)
            .is_some_and(|(_, end)| billed.read.until <= midnight(end));
        if !inside {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "The subscription bills {} from {} to {}, across the agreement's periods: align \
                 its billing day and cadence first",
                billed.row.fee.name,
                day,
                billed.read.until.date_naive()
            ))));
        }
    }
    Ok(())
}

/// The live volume of each of `metrics` over `window`: every member covering it, its own usage.
pub async fn window_bases(
    conn: &mut PgConn,
    client: &dyn UsageClient,
    tenant_id: TenantId,
    agreement_id: VolumeAgreementId,
    metrics: &[BillableMetricId],
    window: Window,
) -> StoreResult<Vec<(BillableMetricId, Decimal)>> {
    let members =
        db(
            VolumeAgreementMemberRow::covering(conn, tenant_id, agreement_id, window.0, window.1)
                .await,
        )?;
    let mut loaded = Vec::with_capacity(members.len());
    for member in &members {
        loaded.push(
            Box::pin(crate::services::evaluate::load::load_facts_unpooled(
                conn,
                tenant_id,
                member.subscription_id,
                window.1,
            ))
            .await?,
        );
    }
    let mut bases = Vec::with_capacity(metrics.len());
    for metric in metrics {
        let mut basis = Decimal::ZERO;
        for facts in &loaded {
            basis += member_usage(client, facts, *metric, window).await?;
        }
        bases.push((*metric, basis));
    }
    Ok(bases)
}

impl crate::services::ServicesEdge {
    /// The live volume of the agreement's pooled metrics over its window holding `day`.
    pub async fn volume_agreement_live_bases(
        &self,
        tenant_id: TenantId,
        agreement_id: VolumeAgreementId,
        day: NaiveDate,
    ) -> StoreResult<Option<(Window, Vec<(BillableMetricId, Decimal)>)>> {
        let mut conn = self.store.get_conn().await?;
        let agreement = db(VolumeAgreementRow::find(&mut conn, tenant_id, agreement_id).await)?;
        let Some(window) = agreement_grid(&agreement).and_then(|g| g.period_containing(day)) else {
            return Ok(None);
        };
        let metrics: Vec<BillableMetricId> =
            db(VolumeAgreementMetricRow::list_of(&mut conn, &[agreement_id]).await)?
                .into_iter()
                .map(|m| m.billable_metric_id)
                .collect();
        let bases = window_bases(
            &mut conn,
            self.services.usage_client.as_ref(),
            tenant_id,
            agreement_id,
            &metrics,
            window,
        )
        .await?;
        Ok(Some((window, bases)))
    }
}

/// After a change on a member: refused when a pooled fee no longer bills the agreement's windows
/// from the open period on (ADR-0021 §3).
pub async fn recheck_memberships_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<()> {
    let Some(pool) = load_memberships(conn, tenant_id, subscription_id).await? else {
        return Ok(());
    };
    let today = common_clock::now().date_naive();
    let mut loaded = None;
    for membership in &pool.memberships {
        let Some(grid) = agreement_grid(&membership.agreement) else {
            continue;
        };
        let open = grid
            .period_containing(today)
            .map_or(membership.from, |(start, _)| start.max(membership.from));
        if membership.until.is_some_and(|until| until <= open) {
            continue;
        }
        if loaded.is_none() {
            loaded = Some(
                Box::pin(crate::services::evaluate::load::load_facts_unpooled(
                    conn,
                    tenant_id,
                    subscription_id,
                    today,
                ))
                .await?,
            );
        }
        if let Some(facts) = &loaded {
            check_windows(facts, &grid, &membership.metrics, open, membership.until).map_err(
                |err| {
                    err.attach(format!(
                        "the subscription is in the volume agreement {}",
                        membership.agreement.name
                    ))
                },
            )?;
        }
    }
    Ok(())
}
