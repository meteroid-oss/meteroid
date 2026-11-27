//! One resolution of a subscription's next renewal (R-160), and the notices sent before it: the
//! team's `subscription.renewal_upcoming` at each checkpoint (R-157) and the customer's reminder
//! email (R-161). Term renewals are noticed wherever terms exist; the other sources only while the
//! tenant has retention, which the caller resolves.
use std::collections::HashMap;

use chrono::NaiveDate;
use common_domain::ids::{OrganizationId, SubscriptionId, TenantId};
use diesel_models::query::renewals as query;
use diesel_models::renewals::{RenewalCandidateRow, RenewalNoticeRow, RenewalSettingsRow};
use diesel_models::subscription_phases::SubscriptionTermRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::domain::customer_emails::{EmailContext, EmailKind};
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::renewals::{
    NoticeAudience, Renewal, RenewalMode, RenewalNotice, RenewalSettings, RenewalSource,
    RenewalTermSnapshot, RenewalUpcomingEvent,
};
use crate::errors::StoreError;
use crate::services::subscriptions::terms;
use crate::{Store, StoreResult};

const MAX_CHECKPOINTS: usize = 6;
const MAX_DAYS: u32 = 365;

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::<Report<StoreError>>::into)
}

/// Whether the tenant's plan opens the retention gate; resolved once per tenant and pass.
#[async_trait::async_trait]
pub trait RetentionGate: Send + Sync {
    async fn open(&self, tenant_id: TenantId, organization_id: OrganizationId) -> bool;
}

fn term_of(row: &RenewalCandidateRow) -> Option<SubscriptionTermRow> {
    Some(SubscriptionTermRow {
        subscription_id: row.subscription_id,
        tenant_id: row.tenant_id,
        term_end: row.term_end?,
        auto_renew: row.term_auto_renew?,
        renewal_months: row.term_renewal_months?,
        uplift_percent: row.term_uplift_percent,
        notice_days: row.term_notice_days?,
        renewal_phase_id: row.term_renewal_phase_id,
        created_by: None,
        updated_at: chrono::NaiveDateTime::MIN,
    })
}

/// The next date after `today` that renews or ends the subscription (R-160).
pub fn next_renewal(row: &RenewalCandidateRow, today: NaiveDate) -> Option<Renewal> {
    let term = term_of(row).map(|term| terms::to_domain(&term, today));
    let renewal =
        |date,
         mode,
         source,
         term: Option<&crate::domain::subscription_phases::SubscriptionTerm>| {
            let snapshot = term.map(|term| RenewalTermSnapshot {
                auto_renew: term.auto_renew,
                renewal_months: term.renewal_months,
                uplift_percent: term.uplift_percent,
                notice_days: term.notice_days,
            });
            let opt_out_deadline = term
                .filter(|term| {
                    mode == RenewalMode::Renewing && term.auto_renew && term.notice_days > 0
                })
                .map(|term| term.notice_cutoff);
            Renewal {
                tenant_id: row.tenant_id,
                subscription_id: row.subscription_id,
                customer_id: row.customer_id,
                customer_name: row.customer_name.clone(),
                plan_name: row.plan_name.clone(),
                currency: row.currency.clone(),
                mrr_cents: row.mrr_cents,
                date,
                mode,
                source,
                opt_out_deadline,
                term: snapshot,
            }
        };
    let term = term.filter(|term| term.term_end > today);
    if let Some(cancel) = row.cancel_date.filter(|d| *d > today)
        && term.as_ref().is_none_or(|term| cancel <= term.term_end)
    {
        return Some(renewal(
            cancel,
            RenewalMode::Cancelling,
            RenewalSource::Cancellation,
            None,
        ));
    }
    if let Some(term) = term {
        if row.term_written_by_trial {
            return None;
        }
        let mode = if term.auto_renew {
            RenewalMode::Renewing
        } else {
            RenewalMode::Ending
        };
        return Some(renewal(
            term.term_end,
            mode,
            RenewalSource::Term,
            Some(&term),
        ));
    }
    if row.term_end.is_some() {
        return None;
    }
    if let Some(end) = row.end_date.filter(|d| *d > today) {
        return Some(renewal(
            end,
            RenewalMode::Ending,
            RenewalSource::EndDate,
            None,
        ));
    }
    let long_period = matches!(row.period.as_str(), "ANNUAL" | "SEMIANNUAL");
    row.current_period_end
        .filter(|d| long_period && *d > today)
        .map(|d| renewal(d, RenewalMode::Renewing, RenewalSource::Period, None))
}

/// Days from `today` to the renewal's anchor, none once its date is reached; zero once its
/// opt-out deadline has passed.
pub fn days_to_anchor(renewal: &Renewal, today: NaiveDate) -> Option<u32> {
    (today < renewal.date)
        .then(|| u32::try_from((renewal.anchor() - today).num_days().max(0)).unwrap_or(0))
}

/// The checkpoint `days_left` has reached: the nearest not past. Checkpoints already behind when
/// tracking starts are skipped, so a renewal 20 days out sends one notice, not three.
pub fn reached_checkpoint(checkpoints: &[u32], days_left: u32) -> Option<u32> {
    checkpoints
        .iter()
        .copied()
        .filter(|c| *c >= days_left)
        .min()
}

/// The customer's reminder is due from `lead` days before the anchor until the anchor itself.
fn reminder_due(renewal: &Renewal, today: NaiveDate, lead: Option<u32>) -> Option<u32> {
    let lead = lead?;
    if renewal.mode == RenewalMode::Cancelling || today > renewal.anchor() || today >= renewal.date
    {
        return None;
    }
    let days = u32::try_from((renewal.anchor() - today).num_days()).ok()?;
    (days <= lead).then_some(lead)
}

fn settings_of(row: Option<RenewalSettingsRow>) -> RenewalSettings {
    row.map(|row| RenewalSettings {
        checkpoints: row
            .checkpoints
            .into_iter()
            .flatten()
            .filter_map(|d| u32::try_from(d).ok())
            .collect(),
        customer_reminder_days: row
            .customer_reminder_days
            .and_then(|d| u32::try_from(d).ok()),
    })
    .unwrap_or_default()
}

pub async fn get_settings(store: &Store, tenant_id: TenantId) -> StoreResult<RenewalSettings> {
    let mut conn = store.get_conn().await?;
    Ok(settings_of(db(
        query::get_settings(&mut conn, tenant_id).await
    )?))
}

pub async fn update_settings(
    store: &Store,
    tenant_id: TenantId,
    checkpoints: Option<Vec<u32>>,
    customer_reminder_days: Option<Option<u32>>,
) -> StoreResult<RenewalSettings> {
    let invalid = |message: &str| Report::new(StoreError::InvalidArgument(message.to_string()));
    let current = get_settings(store, tenant_id).await?;
    let mut checkpoints = checkpoints.unwrap_or(current.checkpoints);
    checkpoints.sort_unstable_by(|a, b| b.cmp(a));
    checkpoints.dedup();
    if checkpoints.len() > MAX_CHECKPOINTS {
        return Err(invalid("at most 6 renewal checkpoints"));
    }
    if checkpoints.iter().any(|d| !(1..=MAX_DAYS).contains(d)) {
        return Err(invalid("a renewal checkpoint is 1 to 365 days"));
    }
    let customer_reminder_days = customer_reminder_days.unwrap_or(current.customer_reminder_days);
    if customer_reminder_days.is_some_and(|d| !(1..=MAX_DAYS).contains(&d)) {
        return Err(invalid("a customer reminder is 1 to 365 days ahead"));
    }
    let row = RenewalSettingsRow {
        tenant_id,
        checkpoints: checkpoints.iter().map(|d| Some(*d as i32)).collect(),
        customer_reminder_days: customer_reminder_days.map(|d| d as i32),
        updated_at: common_clock::now(),
    };
    let mut conn = store.get_conn().await?;
    Ok(settings_of(Some(db(query::upsert_settings(
        &mut conn, &row,
    )
    .await)?)))
}

/// The tenant's renewals within `within_days`, soonest first; `period_sources` includes those not
/// dated by a term (the retention gate).
pub async fn list_upcoming(
    store: &Store,
    tenant_id: TenantId,
    within_days: u32,
    today: NaiveDate,
    period_sources: bool,
) -> StoreResult<Vec<Renewal>> {
    let mut conn = store.get_conn().await?;
    let rows = db(query::list_renewal_candidates(&mut conn, Some(&[tenant_id]), None).await)?;
    let mut renewals: Vec<_> = rows
        .iter()
        .filter_map(|row| next_renewal(row, today))
        .filter(|r| period_sources || !r.source.needs_retention())
        .filter(|r| (r.date - today).num_days() <= i64::from(within_days))
        .collect();
    renewals.sort_by_key(|r| (r.date, r.subscription_id));
    Ok(renewals)
}

/// A subscription's next renewal, whatever its source, and the notices sent for it so far.
pub async fn subscription_renewal(
    store: &Store,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    today: NaiveDate,
) -> StoreResult<(Option<Renewal>, Vec<RenewalNotice>)> {
    let mut conn = store.get_conn().await?;
    let renewal =
        db(
            query::list_renewal_candidates(&mut conn, Some(&[tenant_id]), Some(subscription_id))
                .await,
        )?
        .first()
        .and_then(|row| next_renewal(row, today));
    let notices = db(query::list_notices(&mut conn, tenant_id, subscription_id).await)?
        .into_iter()
        .filter_map(|row| {
            Some(RenewalNotice {
                renewal_date: row.renewal_date,
                mode: RenewalMode::parse(&row.mode)?,
                audience: NoticeAudience::parse(&row.audience)?,
                days_before: u32::try_from(row.days_before).ok()?,
                notified_at: row.notified_at,
            })
        })
        .collect();
    Ok((renewal, notices))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepReport {
    pub team: usize,
    pub customer: usize,
}

struct Due {
    renewal: Renewal,
    team: Option<u32>,
    customer: Option<u32>,
}

/// Sends the notices due on `today` (R-157, R-161). Replicas may run it together: the claim of
/// each notice, written with its event or email, decides which one sends it.
pub async fn sweep(
    store: &Store,
    gate: &dyn RetentionGate,
    today: NaiveDate,
) -> StoreResult<SweepReport> {
    let rows = {
        let mut conn = store.get_conn().await?;
        db(query::list_renewal_candidates(&mut conn, None, None).await)?
    };
    let mut retention: HashMap<TenantId, bool> = HashMap::new();
    let mut settings: HashMap<TenantId, RenewalSettings> = HashMap::new();
    let mut report = SweepReport::default();
    for row in &rows {
        let Some(renewal) = next_renewal(row, today) else {
            continue;
        };
        if renewal.source.needs_retention() {
            let open = match retention.get(&row.tenant_id) {
                Some(open) => *open,
                None => {
                    let open = gate.open(row.tenant_id, row.organization_id).await;
                    retention.insert(row.tenant_id, open);
                    open
                }
            };
            if !open {
                continue;
            }
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = settings.entry(row.tenant_id) {
            entry.insert(get_settings(store, row.tenant_id).await?);
        }
        let policy = &settings[&row.tenant_id];
        let Some(days_left) = days_to_anchor(&renewal, today) else {
            continue;
        };
        let due = Due {
            team: reached_checkpoint(&policy.checkpoints, days_left),
            customer: reminder_due(&renewal, today, policy.customer_reminder_days),
            renewal,
        };
        if due.team.is_none() && due.customer.is_none() {
            continue;
        }
        let (team, customer) = notify(store, due).await?;
        report.team += usize::from(team);
        report.customer += usize::from(customer);
    }
    Ok(report)
}

fn claim(renewal: &Renewal, audience: NoticeAudience, days_before: u32) -> RenewalNoticeRow {
    RenewalNoticeRow {
        subscription_id: renewal.subscription_id,
        tenant_id: renewal.tenant_id,
        renewal_date: renewal.date,
        days_before: i32::try_from(days_before).unwrap_or(i32::MAX),
        mode: renewal.mode.as_str().to_string(),
        audience: audience.as_str().to_string(),
        notified_at: common_clock::now(),
    }
}

async fn notify(store: &Store, due: Due) -> StoreResult<(bool, bool)> {
    store
        .transaction(|conn| {
            async move {
                let renewal = &due.renewal;
                let mut sent = (false, false);
                if let Some(days_before) = due.team
                    && db(query::claim_notice(
                        conn,
                        &claim(renewal, NoticeAudience::Team, days_before),
                    )
                    .await)?
                {
                    let event = RenewalUpcomingEvent {
                        id: RenewalUpcomingEvent::event_id(
                            renewal.subscription_id,
                            renewal.date,
                            days_before,
                            renewal.mode,
                        ),
                        tenant_id: renewal.tenant_id,
                        customer_id: renewal.customer_id,
                        subscription_id: renewal.subscription_id,
                        renewal_date: renewal.date,
                        mode: renewal.mode,
                        source: renewal.source,
                        days_before,
                        opt_out_deadline: renewal.opt_out_deadline,
                        term: renewal.term.clone(),
                        plan_name: renewal.plan_name.clone(),
                        currency: renewal.currency.clone(),
                        mrr_cents: renewal.mrr_cents,
                        notified_at: common_clock::now(),
                    };
                    crate::repositories::outbox::publish_outbox_events_tx(
                        conn,
                        vec![OutboxEvent::renewal_upcoming(event)],
                    )
                    .await?;
                    sent.0 = true;
                }
                if let Some(days_before) = due.customer
                    && db(query::claim_notice(
                        conn,
                        &claim(renewal, NoticeAudience::Customer, days_before),
                    )
                    .await)?
                {
                    let kind = EmailKind::RenewalReminder;
                    crate::repositories::customer_emails::enqueue_customer_email_on_tx(
                        conn,
                        renewal.tenant_id,
                        kind,
                        EmailContext::RenewalReminder {
                            customer_id: renewal.customer_id,
                            subscription_id: renewal.subscription_id,
                            plan_name: renewal.plan_name.clone(),
                            date: renewal.date,
                            renews: renewal.mode == RenewalMode::Renewing,
                            opt_out_deadline: renewal.opt_out_deadline,
                            renewal_months: renewal.term.as_ref().map(|t| t.renewal_months),
                            uplift_percent: renewal
                                .term
                                .as_ref()
                                .and_then(|t| t.uplift_percent)
                                .filter(|p| !p.is_zero()),
                        },
                        format!(
                            "{kind}:{}:{}:{}",
                            renewal.subscription_id,
                            renewal.date,
                            renewal.mode.as_str()
                        ),
                    )
                    .await?;
                    sent.1 = true;
                }
                Ok(sent)
            }
            .scope_boxed()
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn d(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    fn row(period: &str, period_end: Option<NaiveDate>) -> RenewalCandidateRow {
        RenewalCandidateRow {
            tenant_id: Uuid::nil().into(),
            organization_id: Uuid::nil().into(),
            subscription_id: Uuid::nil().into(),
            customer_id: Uuid::nil().into(),
            customer_name: String::new(),
            plan_name: String::new(),
            currency: "EUR".into(),
            mrr_cents: 0,
            period: period.into(),
            current_period_end: period_end,
            end_date: None,
            cancel_date: None,
            term_end: None,
            term_auto_renew: None,
            term_renewal_months: None,
            term_uplift_percent: None,
            term_notice_days: None,
            term_renewal_phase_id: None,
            term_written_by_trial: false,
        }
    }

    fn with_term(
        mut row: RenewalCandidateRow,
        end: NaiveDate,
        renews: bool,
        notice: i32,
    ) -> RenewalCandidateRow {
        row.term_end = Some(end);
        row.term_auto_renew = Some(renews);
        row.term_renewal_months = Some(12);
        row.term_notice_days = Some(notice);
        if !renews {
            row.end_date = Some(end);
        }
        row
    }

    fn at(
        row: &RenewalCandidateRow,
        today: NaiveDate,
    ) -> Option<(NaiveDate, RenewalMode, RenewalSource)> {
        next_renewal(row, today).map(|r| (r.date, r.mode, r.source))
    }

    #[test]
    fn periods_count_only_without_a_term() {
        let today = d(3, 1);
        assert_eq!(
            at(&row("ANNUAL", Some(d(5, 1))), today),
            Some((d(5, 1), RenewalMode::Renewing, RenewalSource::Period))
        );
        assert_eq!(at(&row("MONTHLY", Some(d(4, 1))), today), None);
        let termed = with_term(row("ANNUAL", Some(d(5, 1))), d(11, 1), true, 30);
        let renewal = next_renewal(&termed, today).unwrap();
        assert_eq!(
            (renewal.date, renewal.mode, renewal.source),
            (d(11, 1), RenewalMode::Renewing, RenewalSource::Term)
        );
        assert_eq!(renewal.opt_out_deadline, Some(d(10, 2)));
        assert_eq!(renewal.anchor(), d(10, 2));
    }

    #[test]
    fn a_term_not_renewing_ends_and_hides_its_end_date() {
        let termed = with_term(row("MONTHLY", Some(d(4, 1))), d(6, 1), false, 0);
        assert_eq!(
            at(&termed, d(3, 1)),
            Some((d(6, 1), RenewalMode::Ending, RenewalSource::Term))
        );
        assert_eq!(
            next_renewal(&termed, d(3, 1)).unwrap().opt_out_deadline,
            None
        );
        let mut ending = row("MONTHLY", Some(d(4, 1)));
        ending.end_date = Some(d(6, 1));
        assert_eq!(
            at(&ending, d(3, 1)),
            Some((d(6, 1), RenewalMode::Ending, RenewalSource::EndDate))
        );
    }

    #[test]
    fn a_cancellation_before_the_term_end_comes_first() {
        let mut termed = with_term(row("MONTHLY", Some(d(4, 1))), d(9, 1), true, 30);
        termed.cancel_date = Some(d(4, 1));
        assert_eq!(
            at(&termed, d(3, 1)),
            Some((
                d(4, 1),
                RenewalMode::Cancelling,
                RenewalSource::Cancellation
            ))
        );
        termed.cancel_date = Some(d(10, 1));
        assert_eq!(at(&termed, d(3, 1)).unwrap().1, RenewalMode::Renewing);
    }

    #[test]
    fn a_trial_term_is_left_to_the_trial() {
        let mut trial = with_term(row("MONTHLY", None), d(3, 15), false, 0);
        trial.term_written_by_trial = true;
        assert_eq!(at(&trial, d(3, 1)), None);
    }

    #[test]
    fn a_term_rolling_on_unwritten_renews_at_its_current_end() {
        let termed = with_term(row("MONTHLY", None), d(1, 15), true, 0);
        assert_eq!(
            at(&termed, d(3, 1)).map(|(date, ..)| date),
            Some(NaiveDate::from_ymd_opt(2027, 1, 15).unwrap())
        );
    }

    #[test]
    fn checkpoints_count_to_the_deadline_and_clamp_past_it() {
        let termed = with_term(row("MONTHLY", None), d(12, 1), true, 90);
        let renewal = next_renewal(&termed, d(3, 1)).unwrap();
        assert_eq!(renewal.anchor(), d(9, 2));
        assert_eq!(days_to_anchor(&renewal, d(8, 3)), Some(30));
        assert_eq!(days_to_anchor(&renewal, d(10, 1)), Some(0));
        assert_eq!(reached_checkpoint(&[90, 60, 30], 0), Some(30));
        assert_eq!(days_to_anchor(&renewal, d(12, 1)), None);
    }

    #[test]
    fn the_nearest_checkpoint_not_past_is_reached() {
        let checkpoints = crate::domain::renewals::DEFAULT_CHECKPOINTS;
        assert_eq!(reached_checkpoint(&checkpoints, 120), None);
        assert_eq!(reached_checkpoint(&checkpoints, 90), Some(90));
        assert_eq!(reached_checkpoint(&checkpoints, 61), Some(90));
        assert_eq!(reached_checkpoint(&checkpoints, 20), Some(30));
        assert_eq!(reached_checkpoint(&[], 20), None);
    }

    #[test]
    fn the_customer_is_reminded_until_the_deadline_only() {
        let termed = with_term(row("MONTHLY", None), d(12, 1), true, 30);
        let renewal = next_renewal(&termed, d(3, 1)).unwrap();
        assert_eq!(reminder_due(&renewal, d(10, 1), Some(30)), None);
        assert_eq!(reminder_due(&renewal, d(10, 2), Some(30)), Some(30));
        assert_eq!(reminder_due(&renewal, d(11, 1), Some(30)), Some(30));
        assert_eq!(reminder_due(&renewal, d(11, 2), Some(30)), None);
        assert_eq!(reminder_due(&renewal, d(10, 2), None), None);
        let mut cancelling = row("MONTHLY", None);
        cancelling.cancel_date = Some(d(4, 1));
        let renewal = next_renewal(&cancelling, d(3, 25)).unwrap();
        assert_eq!(reminder_due(&renewal, d(3, 25), Some(30)), None);
    }
}
