//! One change engine (ADR 0015, R-162): a change dated `from` on a subscription with timeline
//! changes on or after it lifts them (withdrawn newest first, their owners kept), is written, then
//! the timeline is replayed from its owners in date order over what it leaves.
use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, ScheduledEventId, SubscriptionId, TenantId};
use diesel_models::PgConn;
use diesel_models::enums::SubscriptionChangeKindEnum;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscription_phases::{SubscriptionTermRow, SubscriptionTimelineItemRow};
use error_stack::Report;

use crate::StoreResult;
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use crate::domain::subscription_phases::{
    RewriteEffect, RewriteEntry, SubscriptionPhase, SubscriptionPhaseInput, TimelineOwner,
};
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::subscriptions::phases::{PhaseWrite, list_phases_tx, write_phase_as_tx};
use crate::services::subscriptions::timeline_items::{ChangeRows, schedule_replayed_tx};

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::into)
}

/// An item's boundary lifted: its day and the rows it closes and opens.
struct LiftedChange {
    on: NaiveDate,
    rows: ChangeRows,
}

struct LiftedItem {
    row: SubscriptionTimelineItemRow,
    start: Option<LiftedChange>,
    end: Option<LiftedChange>,
}

/// The timeline changes on or after `from`, withdrawn, with what is needed to write them again.
pub(crate) struct Lifted {
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    phases: Vec<SubscriptionPhase>,
    items: Vec<LiftedItem>,
    term: Option<SubscriptionTermRow>,
}

impl Lifted {
    pub fn is_empty(&self) -> bool {
        self.phases.is_empty() && self.items.is_empty() && self.term.is_none()
    }

    /// The lifted phases, which the caller writes again itself (set_phases replaces them).
    pub fn take_phases(&mut self) -> Vec<SubscriptionPhase> {
        std::mem::take(&mut self.phases)
    }

    /// The term the caller detached already, attached back after the replay.
    pub fn set_term(&mut self, term: Option<SubscriptionTermRow>) {
        if term.is_some() {
            self.term = term;
        }
    }

    /// The phase switching plan on `day`, made to switch to `plan` at `prices` instead: a plan
    /// change of its day replaces its switch (R-162). Its id, when there is one.
    pub fn replace_switch_on(
        &mut self,
        day: NaiveDate,
        plan: crate::domain::subscription_phases::PhasePlan,
        prices: &[crate::domain::subscription_changes::TargetPrice],
    ) -> Option<common_domain::ids::SubscriptionPhaseId> {
        let phase = self
            .phases
            .iter_mut()
            .find(|phase| phase.start_date == day && phase.terms.plan.is_some())?;
        phase.terms.plan = Some(plan);
        phase.terms.components = prices
            .iter()
            .map(|price| crate::domain::subscription_phases::PhaseComponent {
                lineage: None,
                plan_component: Some(price.component_id),
                price: price.price,
                list_rate: None,
                name: None,
            })
            .collect();
        Some(phase.id)
    }

    /// The phases switching plan on or before `day` withdrawn, `why` reported: a plan change in a
    /// trial ends it, and the switch written for its end with it.
    pub async fn drop_switches_until(
        &mut self,
        conn: &mut PgConn,
        day: NaiveDate,
        why: &str,
    ) -> StoreResult<Vec<RewriteEntry>> {
        let (dropped, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.phases)
            .into_iter()
            .partition(|phase| phase.terms.plan.is_some() && phase.start_date <= day);
        self.phases = kept;
        let mut entries = Vec::new();
        for phase in dropped {
            db(
                diesel_models::subscription_phases::SubscriptionPhaseRow::withdraw(conn, phase.id)
                    .await,
            )?;
            entries.push(RewriteEntry {
                owner: TimelineOwner::Phase(phase.id),
                label: phase
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("Phase {}", phase.seq + 1)),
                on: phase.start_date,
                effect: RewriteEffect::Dropped(why.to_string()),
            });
        }
        Ok(entries)
    }
}

/// The entry of `phase` marked as its switch replaced.
pub(crate) fn mark_switch_replaced(
    entries: &mut [RewriteEntry],
    phase: common_domain::ids::SubscriptionPhaseId,
) {
    for entry in entries
        .iter_mut()
        .filter(|entry| entry.owner == TimelineOwner::Phase(phase))
    {
        entry.effect = RewriteEffect::SwitchReplaced;
    }
}

/// The change a live phase's boundary is now.
pub(crate) async fn phase_change_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    phase: common_domain::ids::SubscriptionPhaseId,
) -> StoreResult<ScheduledEvent> {
    let change_id = list_phases_tx(conn, tenant_id, subscription_id)
        .await?
        .into_iter()
        .find(|live| live.id == phase)
        .map(|live| live.change_id)
        .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("phase {phase}"))))?;
    ScheduledEvent::try_from(db(SubscriptionChangeRow::get_by_id(
        conn, change_id, &tenant_id,
    )
    .await)?)
}

/// The subscription's timeline changes still to apply on or after `from` withdrawn, newest
/// first; their phases, items and term stay, to be replayed by [`replay_tx`].
pub(crate) async fn lift_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    from: NaiveDate,
) -> StoreResult<Lifted> {
    let today = common_clock::now().date_naive();
    let ahead = |day: NaiveDate| day >= from && day > today;
    let term = match db(SubscriptionTermRow::find(conn, &tenant_id, subscription_id).await)? {
        Some(term) if term.renewal_phase_id.is_some() && ahead(term.term_end) => {
            crate::services::subscriptions::terms::detach_renewal_tx(
                conn,
                tenant_id,
                subscription_id,
                &[],
            )
            .await?
        }
        _ => None,
    };

    let mut changes: Vec<SubscriptionChangeRow> = Vec::new();
    let phases: Vec<SubscriptionPhase> = list_phases_tx(conn, tenant_id, subscription_id)
        .await?
        .into_iter()
        .filter(|phase| ahead(phase.start_date))
        .collect();
    for phase in &phases {
        changes.push(db(SubscriptionChangeRow::get_by_id(
            conn,
            phase.change_id,
            &tenant_id,
        )
        .await)?);
    }
    let mut items = Vec::new();
    for row in db(SubscriptionTimelineItemRow::list_live(conn, &tenant_id, subscription_id).await)?
    {
        let mut lifted = LiftedItem {
            start: None,
            end: None,
            row: row.clone(),
        };
        for (change_id, day, is_start) in [
            (row.start_change_id, row.start_date, true),
            (Some(row.end_change_id), row.until_date, false),
        ] {
            let Some(change_id) = change_id.filter(|_| ahead(day)) else {
                continue;
            };
            let change = db(SubscriptionChangeRow::get_by_id(conn, change_id, &tenant_id).await)?;
            if change.withdrawn_at.is_some() {
                continue;
            }
            let event = ScheduledEvent::try_from(change.clone())?;
            let ScheduledEventData::ApplyPhase {
                component_close,
                component_insert,
                addon_close,
                addon_insert,
                ..
            } = event.event_data
            else {
                continue;
            };
            let boundary = LiftedChange {
                on: day,
                rows: ChangeRows {
                    component_close,
                    component_insert,
                    addon_close,
                    addon_insert,
                },
            };
            if is_start {
                lifted.start = Some(boundary);
            } else {
                lifted.end = Some(boundary);
            }
            changes.push(change);
        }
        if lifted.start.is_some() || lifted.end.is_some() {
            items.push(lifted);
        }
    }

    changes.retain(|change| change.withdrawn_at.is_none());
    changes.sort_by_key(|change| std::cmp::Reverse((change.effective_at, change.created_at)));
    for change in &changes {
        crate::services::facts::pending::withdraw_pending_tx(
            conn,
            change,
            "Rewritten over an earlier change",
        )
        .await?;
    }
    Ok(Lifted {
        tenant_id,
        subscription_id,
        phases,
        items,
        term,
    })
}

enum Boundary {
    ItemEnd(usize),
    Phase(usize),
    Authored(usize),
    ItemStart(usize),
}

/// The lifted timeline written again in date order (an item's end, then the phases, then an
/// item's start on one day) over the facts as they now stand, each owner reported (R-162).
pub(in crate::services) async fn replay_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    lifted: Lifted,
) -> StoreResult<Vec<RewriteEntry>> {
    replay_with_tx(services, conn, actor, lifted, Vec::new()).await
}

/// [`replay_tx`] with new phases written as asked among them, each at its `seq` (set_phases).
pub(in crate::services) async fn replay_with_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    lifted: Lifted,
    authored: Vec<(u32, SubscriptionPhaseInput)>,
) -> StoreResult<Vec<RewriteEntry>> {
    let (tenant_id, subscription_id) = (lifted.tenant_id, lifted.subscription_id);
    let mut boundaries: Vec<(NaiveDate, u8, Boundary)> = Vec::new();
    for (i, phase) in lifted.phases.iter().enumerate() {
        boundaries.push((phase.start_date, 1, Boundary::Phase(i)));
    }
    for (i, (_, input)) in authored.iter().enumerate() {
        boundaries.push((input.start_date, 1, Boundary::Authored(i)));
    }
    for (i, item) in lifted.items.iter().enumerate() {
        if let Some(start) = &item.start {
            boundaries.push((start.on, 2, Boundary::ItemStart(i)));
        }
        if let Some(end) = &item.end {
            boundaries.push((end.on, 0, Boundary::ItemEnd(i)));
        }
    }
    boundaries.sort_by_key(|(on, order, _)| (*on, *order));

    let mut entries = Vec::new();
    let mut written: Vec<(Option<ScheduledEventId>, Option<ScheduledEventId>)> =
        vec![(None, None); lifted.items.len()];
    let mut dropped: Vec<bool> = vec![false; lifted.items.len()];
    let mut lifted = lifted;
    for (on, _, boundary) in boundaries {
        match boundary {
            Boundary::Authored(i) => {
                let (seq, input) = &authored[i];
                crate::services::subscriptions::phases::write_phase_tx(
                    services,
                    conn,
                    actor,
                    tenant_id,
                    subscription_id,
                    *seq,
                    input,
                )
                .await?;
            }
            Boundary::Phase(i) => {
                let phase = &lifted.phases[i];
                let label = phase
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("Phase {}", phase.seq + 1));
                let input = SubscriptionPhaseInput {
                    start_date: phase.start_date,
                    label: phase.label.clone(),
                    terms: phase.terms.clone(),
                };
                let replayed = write_phase_as_tx(
                    services,
                    conn,
                    actor,
                    tenant_id,
                    subscription_id,
                    phase.seq,
                    &input,
                    PhaseWrite::Replay(phase.id),
                )
                .await
                .map_err(|error| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "{label} on {on} cannot be written again over this change: {}",
                        crate::services::subscriptions::timeline_rewrite::reason(&error)
                    )))
                })?;
                entries.push(RewriteEntry {
                    owner: TimelineOwner::Phase(phase.id),
                    label,
                    on,
                    effect: if !replayed.inactive.is_empty() {
                        RewriteEffect::Inactive(replayed.inactive)
                    } else if replayed.prorated {
                        RewriteEffect::Prorated
                    } else {
                        RewriteEffect::Kept
                    },
                });
            }
            Boundary::ItemStart(i) | Boundary::ItemEnd(i) => {
                if dropped[i] {
                    continue;
                }
                let is_start = matches!(boundary, Boundary::ItemStart(_));
                let item = &mut lifted.items[i];
                let change = if is_start {
                    item.start.take()
                } else {
                    item.end.take()
                };
                let Some(mut change) = change else { continue };
                in_force_of_lineages(conn, subscription_id, &mut change).await?;
                match still_resolves(conn, subscription_id, &change).await? {
                    None => {
                        let id = schedule_replayed_tx(
                            services,
                            conn,
                            actor,
                            tenant_id,
                            subscription_id,
                            item.row.id,
                            change.on,
                            change.rows,
                        )
                        .await?;
                        if is_start {
                            written[i].0 = Some(id);
                        } else {
                            written[i].1 = Some(id);
                        }
                    }
                    Some(why) => {
                        for id in [written[i].0, written[i].1].into_iter().flatten() {
                            let change =
                                db(SubscriptionChangeRow::get_by_id(conn, id, &tenant_id).await)?;
                            crate::services::facts::pending::withdraw_pending_tx(
                                conn,
                                &change,
                                "Timeline item dropped",
                            )
                            .await?;
                        }
                        db(SubscriptionTimelineItemRow::withdraw(conn, item.row.id).await)?;
                        dropped[i] = true;
                        entries.push(RewriteEntry {
                            owner: TimelineOwner::Item(item.row.id),
                            label: item_label(&item.row),
                            on: change.on,
                            effect: RewriteEffect::Dropped(why),
                        });
                    }
                }
            }
        }
    }
    for (i, item) in lifted.items.iter().enumerate() {
        if dropped[i] {
            continue;
        }
        let (start, end) = written[i];
        db(SubscriptionTimelineItemRow::rewritten(
            conn,
            item.row.id,
            start.or(item.row.start_change_id),
            end.unwrap_or(item.row.end_change_id),
        )
        .await)?;
        entries.push(RewriteEntry {
            owner: TimelineOwner::Item(item.row.id),
            label: item_label(&item.row),
            on: item.row.start_date,
            effect: RewriteEffect::Kept,
        });
    }
    if let Some(term) = lifted.term {
        let on = term.term_end;
        crate::services::subscriptions::terms::attach_renewal_tx(services, conn, actor, term)
            .await?;
        entries.push(RewriteEntry {
            owner: TimelineOwner::Renewal,
            label: "Renewal".to_string(),
            on,
            effect: RewriteEffect::Kept,
        });
    }
    entries.sort_by_key(|entry| entry.on);
    Ok(entries)
}

fn item_label(row: &SubscriptionTimelineItemRow) -> String {
    row.label.clone().unwrap_or_else(|| {
        if row.kind == crate::domain::subscription_phases::ITEM_DISCOUNT {
            "Dated discount".to_string()
        } else {
            "Dated add-on".to_string()
        }
    })
}

/// The add-on rows `change` closes, each the one of its lineage in force now: a phase before it
/// may have floored the add-on, its row replaced by the phase's (R-146).
async fn in_force_of_lineages(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    change: &mut LiftedChange,
) -> StoreResult<()> {
    if change.rows.addon_close.is_empty() {
        return Ok(());
    }
    let rows = db(SubscriptionAddOnRow::list_fact_rows(conn, subscription_id).await)?;
    for id in &mut change.rows.addon_close {
        let Some(lineage) = rows
            .iter()
            .find(|row| row.id == *id)
            .map(|row| row.lineage_id.unwrap_or(row.id))
        else {
            continue;
        };
        if let Some(open) = rows
            .iter()
            .find(|row| row.lineage_id.unwrap_or(row.id) == lineage && row.effective_to.is_none())
        {
            *id = open.id;
        }
    }
    Ok(())
}

/// Why `change` no longer resolves: a row it closes is no longer open, or an add-on it opens is
/// gone from the plan in force (the change before it closed it).
async fn still_resolves(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    change: &LiftedChange,
) -> StoreResult<Option<String>> {
    if !change.rows.component_close.is_empty() {
        let rows = db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?;
        if let Some(gone) = change.rows.component_close.iter().find(|id| {
            !rows
                .iter()
                .any(|row| row.id == **id && row.effective_to.is_none())
        }) {
            let name = rows
                .iter()
                .find(|row| row.id == *gone)
                .map_or_else(|| "its fee".to_string(), |row| row.name.clone());
            return Ok(Some(format!("{name} is no longer billed as it was")));
        }
    }
    if !change.rows.addon_close.is_empty() {
        let rows = db(SubscriptionAddOnRow::list_fact_rows(conn, subscription_id).await)?;
        if let Some(gone) = change.rows.addon_close.iter().find(|id| {
            !rows
                .iter()
                .any(|row| row.id == **id && row.effective_to.is_none())
        }) {
            let name = rows
                .iter()
                .find(|row| row.id == *gone)
                .map_or_else(|| "its add-on".to_string(), |row| row.name.clone());
            return Ok(Some(format!("{name} is no longer on the subscription")));
        }
    }
    Ok(None)
}

/// A store error's message, for a refusal naming the owner it came from.
pub(crate) fn reason(error: &Report<StoreError>) -> String {
    match error.current_context() {
        StoreError::InvalidArgument(message) | StoreError::FailedPrecondition(message) => {
            message.clone()
        }
        other => other.to_string(),
    }
}

/// The changes made alone of `kinds` still to come withdrawn, the timeline after the earliest
/// written again over what they leave (R-162): a cancellation replacing them (R-141).
pub(in crate::services) async fn withdraw_one_offs_under_timeline_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    kinds: &[SubscriptionChangeKindEnum],
    reason: &str,
) -> StoreResult<Vec<RewriteEntry>> {
    let one_offs = crate::services::subscriptions::phases::pending_one_offs_tx(
        conn,
        tenant_id,
        subscription_id,
        kinds,
    )
    .await?;
    let Some(from) = one_offs
        .iter()
        .map(|change| change.effective_at.date())
        .min()
    else {
        return Ok(Vec::new());
    };
    let lifted = lift_tx(conn, tenant_id, subscription_id, from).await?;
    crate::services::subscriptions::phases::withdraw_one_offs_tx(
        conn,
        tenant_id,
        subscription_id,
        kinds,
        reason,
    )
    .await?;
    replay_tx(services, conn, actor, lifted).await
}

/// Whether the subscription has a timeline change still to apply on or after `from`.
pub(crate) async fn has_timeline_from(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    from: NaiveDate,
) -> StoreResult<bool> {
    let today = common_clock::now().date_naive();
    let ahead = |day: NaiveDate| day >= from && day > today;
    if list_phases_tx(conn, tenant_id, subscription_id)
        .await?
        .iter()
        .any(|phase| ahead(phase.start_date))
    {
        return Ok(true);
    }
    Ok(
        db(SubscriptionTimelineItemRow::list_live(conn, &tenant_id, subscription_id).await)?
            .iter()
            .any(|item| ahead(item.until_date)),
    )
}

/// The months a timeline effect projects: 12, or through the term's end, at most 60.
async fn projection_months(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<u32> {
    use chrono::Datelike;
    let today = common_clock::now().date_naive();
    Ok(
        match db(SubscriptionTermRow::find(conn, &tenant_id, subscription_id).await)? {
            Some(term) => {
                let months = (term.term_end.year() - today.year()) * 12
                    + i32::try_from(term.term_end.month()).unwrap_or(1)
                    - i32::try_from(today.month()).unwrap_or(1)
                    + 1;
                u32::try_from(months.clamp(12, 60)).unwrap_or(12)
            }
            None => 12,
        },
    )
}

/// What `edit`, a change dated `on`, does to the subscription's timeline, written in a
/// transaction rolled back: each owner rewritten and the invoices ahead without and with it
/// (from tomorrow for an immediate change, whose own documents are previewed apart). `None` when
/// no timeline change comes on or after `on`.
pub(in crate::services) async fn effect_of<'a, F>(
    services: &Services,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    immediate: bool,
    edit: F,
) -> StoreResult<Option<crate::domain::subscription_phases::TimelineEffect>>
where
    F: for<'r> FnOnce(
            &'r mut PgConn,
        )
            -> scoped_futures::ScopedBoxFuture<'a, 'r, StoreResult<Vec<RewriteEntry>>>
        + Send
        + 'a,
{
    use scoped_futures::ScopedFutureExt;
    let mut conn = services.store.get_conn().await?;
    if !has_timeline_from(&mut conn, tenant_id, subscription_id, on).await? {
        return Ok(None);
    }
    let months = projection_months(&mut conn, tenant_id, subscription_id).await?;
    let today = common_clock::now().date_naive();
    let from = if immediate {
        today.succ_opt().unwrap_or(today)
    } else {
        today
    };
    let before = crate::services::timeline::project_from_tx(
        &mut conn,
        tenant_id,
        subscription_id,
        months,
        from,
    )
    .await?;
    drop(conn);
    let mut written = None;
    let rolled_back = services
        .store
        .transaction(|conn| {
            let written = &mut written;
            async move {
                let entries = edit(conn).await?;
                let after = crate::services::timeline::project_from_tx(
                    conn,
                    tenant_id,
                    subscription_id,
                    months,
                    from,
                )
                .await?;
                *written = Some((entries, after));
                Err::<(), _>(Report::new(StoreError::FailedPrecondition(
                    "a timeline preview is rolled back".into(),
                )))
            }
            .scope_boxed()
        })
        .await;
    match (written, rolled_back) {
        (Some((entries, after)), _) => {
            Ok(Some(crate::domain::subscription_phases::TimelineEffect {
                entries,
                before,
                after,
            }))
        }
        (None, Err(error)) => Err(error),
        (None, Ok(())) => Err(Report::new(StoreError::FailedPrecondition(
            "a timeline preview was not rolled back".into(),
        ))),
    }
}

impl Services {
    /// What a plan change does to the subscription's timeline, read in a transaction rolled
    /// back: `None` without a timeline after it (R-162).
    pub(in crate::services) async fn plan_change_timeline_effect(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        target: crate::domain::subscription_changes::PlanTarget,
        component_params: Vec<crate::domain::ComponentParameterization>,
        immediate: bool,
    ) -> StoreResult<Option<crate::domain::subscription_phases::TimelineEffect>> {
        use crate::repositories::SubscriptionInterface;
        use scoped_futures::ScopedFutureExt;
        let today = common_clock::now().date_naive();
        let on = if immediate {
            today
        } else {
            let mut conn = self.store.get_conn().await?;
            let details = self
                .store
                .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
                .await?;
            details.subscription.current_period_end.unwrap_or(today)
        };
        effect_of(self, tenant_id, subscription_id, on, immediate, |conn| {
            let services = self;
            let actor = &actor;
            let component_params = &component_params;
            async move {
                if immediate {
                    let version = target.version;
                    let prepared = services
                        .prepare_plan_change_tx(
                            conn,
                            subscription_id,
                            tenant_id,
                            target,
                            component_params,
                            today,
                        )
                        .await?
                        .without_settlement();
                    services
                        .execute_plan_change_reporting_tx(
                            conn,
                            actor,
                            &prepared,
                            subscription_id,
                            tenant_id,
                            version,
                            today,
                        )
                        .await
                        .map(|(_, timeline)| timeline)
                } else {
                    services
                        .schedule_plan_change_tx(
                            conn,
                            actor,
                            subscription_id,
                            tenant_id,
                            target,
                            component_params,
                        )
                        .await
                        .map(|(_, timeline)| timeline)
                }
            }
            .scope_boxed()
        })
        .await
    }
}

/// A timeline change as its rows read: its day, the fees and add-ons it opens (name and fee) and
/// the names of those it closes, so two writings of it compare equal whatever their row ids.
async fn written_as(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    change: &SubscriptionChangeRow,
) -> StoreResult<String> {
    let Some(record) = change.written_facts.clone() else {
        return Ok(format!("{}: nothing written", change.effective_at.date()));
    };
    let written: crate::services::facts::pending::WrittenFacts = serde_json::from_value(record)
        .map_err(|error| Report::new(StoreError::SerdeError("written facts".into(), error)))?;
    let components = db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?;
    let add_ons = db(SubscriptionAddOnRow::list_fact_rows(conn, subscription_id).await)?;
    let named = |ids: &[uuid::Uuid], with_fee: bool| {
        let mut out: Vec<String> = ids
            .iter()
            .map(|id| {
                components
                    .iter()
                    .find(|row| row.id.as_uuid() == *id)
                    .map(|row| (row.name.clone(), row.legacy_fee.clone()))
                    .or_else(|| {
                        add_ons
                            .iter()
                            .find(|row| row.id.as_uuid() == *id)
                            .map(|row| (row.name.clone(), row.legacy_fee.clone()))
                    })
                    .map_or_else(
                        || format!("? {id}"),
                        |(name, fee)| match (with_fee, fee) {
                            (true, Some(fee)) => format!("{name} {fee}"),
                            _ => name,
                        },
                    )
            })
            .collect();
        out.sort();
        out
    };
    Ok(format!(
        "{}: opens {:?}, closes {:?}, minimum {}",
        written.on,
        named(
            &[
                written.opened_components.clone(),
                written.opened_add_ons.clone()
            ]
            .concat(),
            true
        ),
        named(
            &[
                written.closed_components.clone(),
                written.closed_add_ons.clone()
            ]
            .concat(),
            false
        ),
        written.minimum
    ))
}

/// The pending timeline changes of the subscription, as their rows read, in date order.
async fn timeline_as_written(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<String>> {
    let mut changes: Vec<SubscriptionChangeRow> = db(SubscriptionChangeRow::list_pending(
        conn,
        &tenant_id,
        subscription_id,
        &[
            SubscriptionChangeKindEnum::Phase,
            SubscriptionChangeKindEnum::PlanChange,
        ],
    )
    .await)?
    .into_iter()
    .filter(crate::services::subscriptions::phases::is_timeline_change)
    .collect();
    changes.sort_by_key(|change| change.effective_at);
    let mut out = Vec::new();
    for change in &changes {
        out.push(written_as(conn, subscription_id, change).await?);
    }
    out.sort();
    Ok(out)
}

/// Where the subscription's timeline drifts from its owners (I35): each pending timeline change
/// against what replaying its owners over the facts before it writes (read in a transaction
/// rolled back), and each fee or add-on lineage with two rows in force on one day.
pub(in crate::services) async fn timeline_drift(
    services: &Services,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<String>> {
    use scoped_futures::ScopedFutureExt;
    let mut found = None;
    let rolled_back = services
        .store
        .transaction(|conn| {
            let found = &mut found;
            async move {
                let mut drift = Vec::new();
                let components =
                    db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?;
                let add_ons =
                    db(SubscriptionAddOnRow::list_fact_rows(conn, subscription_id).await)?;
                let rows: Vec<(String, uuid::Uuid, NaiveDate, Option<NaiveDate>)> = components
                    .iter()
                    .map(|row| {
                        (
                            row.name.clone(),
                            row.lineage_id.unwrap_or(row.id).as_uuid(),
                            row.effective_from,
                            row.effective_to,
                        )
                    })
                    .chain(add_ons.iter().map(|row| {
                        (
                            row.name.clone(),
                            row.lineage_id.unwrap_or(row.id).as_uuid(),
                            row.effective_from,
                            row.effective_to,
                        )
                    }))
                    .filter(|(_, _, from, to)| to.is_none_or(|to| to > *from))
                    .collect();
                for (i, a) in rows.iter().enumerate() {
                    for b in rows.iter().skip(i + 1) {
                        let overlap = a.2 < b.3.unwrap_or(NaiveDate::MAX)
                            && b.2 < a.3.unwrap_or(NaiveDate::MAX);
                        if a.1 == b.1 && overlap {
                            drift.push(format!(
                                "{} has two rows in force from {}",
                                a.0,
                                a.2.max(b.2)
                            ));
                        }
                    }
                }
                let before = timeline_as_written(conn, tenant_id, subscription_id).await?;
                let lifted = lift_tx(conn, tenant_id, subscription_id, NaiveDate::MIN).await?;
                replay_tx(services, conn, actor, lifted).await?;
                let after = timeline_as_written(conn, tenant_id, subscription_id).await?;
                if before != after {
                    drift.push(format!("written {before:#?}, replayed {after:#?}"));
                }
                *found = Some(drift);
                Err::<(), _>(Report::new(StoreError::FailedPrecondition(
                    "a drift check is rolled back".into(),
                )))
            }
            .scope_boxed()
        })
        .await;
    match (found, rolled_back) {
        (Some(drift), _) => Ok(drift),
        (None, Err(error)) => Err(error),
        (None, Ok(())) => Ok(Vec::new()),
    }
}

impl crate::services::ServicesEdge {
    /// Where the subscription's timeline drifts from its phases and items (I35); empty when none.
    pub async fn timeline_drift(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Vec<String>> {
        timeline_drift(&self.services, &Actor::System, tenant_id, subscription_id).await
    }
}
