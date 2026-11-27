//! A subscription's contract term (R-152). Without renewal, its end is the subscription's
//! COMPLETE ending. Renewing, it has none: without uplift the term just rolls on; with one, a
//! renewal phase at the term's end restates each fee at its price plus the uplift, and applying it
//! rolls the term and appends the next. Billing reads the ending and the phases, never this.
use chrono::{Months, NaiveDate};
use common_domain::actor::Actor;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::PgConn;
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscription_phases::SubscriptionTermRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::renewals::NoticeWaiver;
use crate::domain::scheduled_events::PhaseSeats;
use crate::domain::subscription_components::SubscriptionFee;
use crate::domain::subscription_phases::{
    PhaseComponent, PhaseTerms, SubscriptionPhaseInput, SubscriptionTerm, SubscriptionTermInput,
    price_of,
};
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::subscriptions::phases::{fee_of, lineage_of, list_phases_tx};

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::into)
}

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

/// The term's end on or after `today`: a term renewing without uplift rolls on unwritten.
fn current_end(row: &SubscriptionTermRow, today: NaiveDate) -> NaiveDate {
    let mut end = row.term_end;
    if row.auto_renew && row.renewal_phase_id.is_none() {
        let months = Months::new(u32::try_from(row.renewal_months).unwrap_or(1));
        while end < today {
            match end.checked_add_months(months) {
                Some(next) => end = next,
                None => break,
            }
        }
    }
    end
}

pub(crate) fn to_domain(row: &SubscriptionTermRow, today: NaiveDate) -> SubscriptionTerm {
    let term_end = current_end(row, today);
    let notice_days = u32::try_from(row.notice_days).unwrap_or_default();
    SubscriptionTerm {
        subscription_id: row.subscription_id,
        term_end,
        auto_renew: row.auto_renew,
        renewal_months: u32::try_from(row.renewal_months).unwrap_or(1),
        uplift_percent: row.uplift_percent,
        notice_days,
        notice_cutoff: term_end - chrono::Duration::days(i64::from(notice_days)),
        renewal_phase_id: row.renewal_phase_id,
    }
}

fn uplifted(row: &SubscriptionTermRow) -> Option<Decimal> {
    row.uplift_percent.filter(|percent| !percent.is_zero())
}

impl Services {
    pub(in crate::services) async fn get_subscription_term(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Option<SubscriptionTerm>> {
        let mut conn = self.store.get_conn().await?;
        let today = common_clock::now().date_naive();
        Ok(
            db(SubscriptionTermRow::find(&mut conn, &tenant_id, subscription_id).await)?
                .map(|row| to_domain(&row, today)),
        )
    }

    /// Sets the term: its end on a renewal after today, and whether and how it renews. Turning
    /// renewal off is an opt-out, taken until the notice cutoff of the term renewing.
    pub(in crate::services) async fn set_subscription_term(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        input: SubscriptionTermInput,
        waiver: Option<NoticeWaiver>,
    ) -> StoreResult<SubscriptionTerm> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let input = &input;
                let waiver = waiver.as_ref();
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    let row = set_term_waiving_tx(
                        self,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        input,
                        waiver,
                    )
                    .await?;
                    let today = common_clock::now().date_naive();
                    self.derive_state_tx(
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        common_clock::now(),
                    )
                    .await?;
                    Ok(to_domain(&row, today))
                }
                .scope_boxed()
            })
            .await
    }
}

/// Sets the term inside the caller's transaction (R-152); the caller derives the state.
pub(in crate::services) async fn set_term_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    input: &SubscriptionTermInput,
) -> StoreResult<SubscriptionTermRow> {
    set_term_waiving_tx(
        services,
        conn,
        actor,
        tenant_id,
        subscription_id,
        input,
        None,
    )
    .await
}

/// [`set_term_tx`], an opt-out after the notice cutoff taken when the operator waives the notice.
pub(in crate::services) async fn set_term_waiving_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    input: &SubscriptionTermInput,
    waiver: Option<&NoticeWaiver>,
) -> StoreResult<SubscriptionTermRow> {
    let today = common_clock::now().date_naive();
    validate(input, today)?;
    // The end it sets moves only what R-140 lets an end date move: nothing billed changes.
    let details = crate::repositories::SubscriptionInterface::get_subscription_details_with_conn(
        services.store.as_ref(),
        conn,
        tenant_id,
        subscription_id,
    )
    .await?;
    let complete_on = (!input.auto_renew).then_some(input.term_end);
    if details.subscription.end_date != complete_on {
        crate::services::subscriptions::end_date::check_end_date(
            conn,
            &details,
            complete_on,
            common_clock::now(),
        )
        .await?;
    }
    let existing = db(SubscriptionTermRow::find(conn, &tenant_id, subscription_id).await)?;
    let late_opt_out = existing
        .as_ref()
        .map(|existing| to_domain(existing, today))
        .filter(|term| term.auto_renew && !input.auto_renew && today > term.notice_cutoff);
    match (&late_opt_out, waiver) {
        (Some(term), None) => {
            return Err(Report::new(StoreError::Conflict(format!(
                "the notice period has passed: the term renews on {}, its opt-out deadline was {}; \
                 waive the notice to accept a late opt-out",
                term.term_end, term.notice_cutoff
            ))));
        }
        (None, Some(_)) => {
            return Err(invalid(
                "no notice to waive: only a late opt-out takes a waiver",
            ));
        }
        (Some(_), Some(waiver)) if !(1..=500).contains(&waiver.reason.trim().chars().count()) => {
            return Err(invalid("a waiver gives its reason, at most 500 characters"));
        }
        _ => {}
    }
    if let Some(existing) = &existing
        && let Some(phase) = existing.renewal_phase_id
    {
        withdraw_renewal_tx(conn, tenant_id, subscription_id, phase).await?;
    }
    crate::services::subscriptions::phases::check_on_renewal(
        conn,
        tenant_id,
        subscription_id,
        input.term_end,
        false,
    )
    .await?;
    if let Some(late) = list_phases_tx(conn, tenant_id, subscription_id)
        .await?
        .into_iter()
        .find(|phase| phase.start_date >= input.term_end)
    {
        return Err(invalid(format!(
            "a phase starts on {}, past the term's end",
            late.start_date
        )));
    }
    let row = SubscriptionTermRow {
        subscription_id,
        tenant_id,
        term_end: input.term_end,
        auto_renew: input.auto_renew,
        renewal_months: i32::try_from(input.renewal_months).unwrap_or(i32::MAX),
        uplift_percent: input.uplift_percent,
        notice_days: i32::try_from(input.notice_days).unwrap_or(i32::MAX),
        renewal_phase_id: None,
        created_by: actor.as_uuid(),
        updated_at: common_clock::now().naive_utc(),
    };
    let row = attach_renewal_tx(services, conn, actor, row).await?;
    audit_term_tx(
        services,
        conn,
        actor,
        &details,
        existing.as_ref(),
        &row,
        waiver.filter(|_| late_opt_out.is_some()),
    )
    .await?;
    Ok(row)
}

/// The term's change on the subscription's activity, with the operator's waiver, and its event.
async fn audit_term_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    details: &crate::domain::SubscriptionDetails,
    before: Option<&SubscriptionTermRow>,
    after: &SubscriptionTermRow,
    waiver: Option<&NoticeWaiver>,
) -> StoreResult<()> {
    use crate::domain::entity_activity::{Activity, ActivityType, AuditInput, EntityType};
    use crate::repositories::entity_activity::EntityActivityInterface;
    use common_domain::ids::BaseId;
    let field = |name: &str, from: serde_json::Value, to: serde_json::Value| {
        (from != to).then(|| serde_json::json!({ "field": name, "from": from, "to": to }))
    };
    let value = |row: Option<&SubscriptionTermRow>,
                 pick: fn(&SubscriptionTermRow) -> serde_json::Value| {
        row.map(pick).unwrap_or(serde_json::Value::Null)
    };
    let changes: Vec<_> = [
        (
            "term_end",
            (|r: &SubscriptionTermRow| serde_json::json!(r.term_end)) as fn(&_) -> _,
        ),
        ("auto_renew", |r| serde_json::json!(r.auto_renew)),
        ("renewal_months", |r| serde_json::json!(r.renewal_months)),
        ("uplift_percent", |r| serde_json::json!(r.uplift_percent)),
        ("notice_days", |r| serde_json::json!(r.notice_days)),
    ]
    .into_iter()
    .filter_map(|(name, pick)| field(name, value(before, pick), value(Some(after), pick)))
    .collect();
    if changes.is_empty() {
        return Ok(());
    }
    let mut metadata = serde_json::json!({ "changes": changes });
    if let Some(waiver) = waiver {
        metadata["notice_waived"] = serde_json::json!({ "reason": waiver.reason.trim() });
    }
    let activity = Activity::new(
        ActivityType::EntityUpdated,
        EntityType::Subscription,
        after.subscription_id.as_uuid(),
    )
    .agg_customer(details.subscription.customer_id)
    .with_metadata(metadata);
    services
        .store
        .record_tx(conn, after.tenant_id, actor, AuditInput::Activity(activity))
        .await?;
    services
        .emit_subscription_updated_tx(
            conn,
            after.tenant_id,
            after.subscription_id,
            crate::domain::outbox_event::SubscriptionUpdateType::BillingConfigurationUpdated,
        )
        .await
}

fn validate(input: &SubscriptionTermInput, today: NaiveDate) -> StoreResult<()> {
    if input.term_end <= today {
        return Err(invalid("a term ends after today"));
    }
    if !(1..=120).contains(&input.renewal_months) {
        return Err(invalid("a term renews for 1 to 120 months"));
    }
    if input.notice_days > 366 {
        return Err(invalid("a term's notice is at most a year"));
    }
    if let Some(percent) = input.uplift_percent
        && (percent.is_sign_negative() || percent > Decimal::ONE_HUNDRED)
    {
        return Err(invalid("an uplift is 0 to 100 percent"));
    }
    Ok(())
}

async fn withdraw_renewal_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    phase_id: common_domain::ids::SubscriptionPhaseId,
) -> StoreResult<()> {
    if let Some(phase) = list_phases_tx(conn, tenant_id, subscription_id)
        .await?
        .into_iter()
        .find(|phase| phase.id == phase_id)
    {
        crate::services::subscriptions::phases::withdraw_phase_tx(
            conn,
            tenant_id,
            &phase,
            "Term replaced",
        )
        .await?;
    }
    Ok(())
}

/// The term's renewal withdrawn before the phases are set again; the term to attach it back to.
pub(crate) async fn detach_renewal_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    phases: &[SubscriptionPhaseInput],
) -> StoreResult<Option<SubscriptionTermRow>> {
    let Some(row) = db(SubscriptionTermRow::find(conn, &tenant_id, subscription_id).await)? else {
        return Ok(None);
    };
    let end = current_end(&row, common_clock::now().date_naive());
    if let Some(late) = phases.iter().find(|phase| phase.start_date >= end) {
        return Err(invalid(format!(
            "the term ends on {end}: a phase starting {} comes with a longer term",
            late.start_date
        )));
    }
    if let Some(phase) = row.renewal_phase_id {
        withdraw_renewal_tx(conn, tenant_id, subscription_id, phase).await?;
    }
    Ok(Some(row))
}

/// The term written as its facts: a COMPLETE ending on its end, or, renewing with an uplift, the
/// renewal phase on it.
pub(in crate::services) async fn attach_renewal_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    mut row: SubscriptionTermRow,
) -> StoreResult<SubscriptionTermRow> {
    let (tenant_id, subscription_id) = (row.tenant_id, row.subscription_id);
    let complete_on = (!row.auto_renew).then_some(row.term_end);
    crate::services::facts::record_end_date_tx(
        conn,
        tenant_id,
        subscription_id,
        complete_on,
        actor,
    )
    .await?;
    row.renewal_phase_id = None;
    if row.auto_renew
        && let Some(percent) = uplifted(&row)
    {
        let terms = renewal_terms(conn, tenant_id, subscription_id, row.term_end, percent).await?;
        let seq = list_phases_tx(conn, tenant_id, subscription_id)
            .await?
            .iter()
            .map(|phase| phase.seq + 1)
            .max()
            .unwrap_or(0);
        let phase = crate::services::subscriptions::phases::write_phase_tx(
            services,
            conn,
            actor,
            tenant_id,
            subscription_id,
            seq,
            &SubscriptionPhaseInput {
                start_date: row.term_end,
                label: Some("Renewal".to_string()),
                terms,
            },
        )
        .await?;
        row.renewal_phase_id = Some(phase);
    }
    row.updated_at = common_clock::now().naive_utc();
    db(row.upsert(conn).await)
}

/// Every fee priced by one amount at its price on `on` plus `percent`, and the seat floors in
/// force: terms are absolute, so a renewal restates what it keeps.
async fn renewal_terms(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    percent: Decimal,
) -> StoreResult<PhaseTerms> {
    let factor = (Decimal::ONE_HUNDRED + percent) / Decimal::ONE_HUNDRED;
    let precision = crate::domain::subscription_phases::precision_of(
        &db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
            .subscription
            .currency,
    );
    let mut components = Vec::new();
    let mut min_seats = Vec::new();
    for row in db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?
        .iter()
        .filter(|row| row.effective_to.is_none())
    {
        if row.effective_from > on {
            return Err(invalid(format!(
                "{} changes on {}, after the term's end",
                row.name, row.effective_from
            )));
        }
        let fee = fee_of(row)?;
        if let Some(price) = price_of(&fee) {
            components.push(PhaseComponent {
                lineage: Some(lineage_of(row)),
                plan_component: None,
                price: crate::domain::subscription_phases::scaled_price(price, factor, precision),
                list_rate: None,
                name: Some(row.name.clone()),
            });
        }
        if let SubscriptionFee::Slot {
            unit,
            quantity_id,
            min_slots: Some(slots),
            ..
        } = &fee
        {
            let quantity_id = match quantity_id {
                Some(id) => *id,
                None => {
                    db(
                        diesel_models::managed_quantities::ManagedQuantityRow::find_by_code(
                            conn, tenant_id, unit,
                        )
                        .await,
                    )?
                    .ok_or_else(|| invalid(format!("no quantity coded {unit}")))?
                    .id
                }
            };
            min_seats.push(PhaseSeats {
                quantity_id,
                code: unit.clone(),
                slots: *slots,
            });
        }
    }
    Ok(PhaseTerms {
        components,
        min_seats,
        ..PhaseTerms::default()
    })
}

/// A renewal phase applied by the controller: the term rolls on and, renewing still, the next
/// renewal is appended (R-152).
pub(in crate::services) async fn renewed_tx(
    services: &Services,
    conn: &mut PgConn,
    change: &SubscriptionChangeRow,
) -> StoreResult<()> {
    let Some(phase) = list_phases_tx(conn, change.tenant_id, change.subscription_id)
        .await?
        .into_iter()
        .find(|phase| phase.change_id == change.id)
    else {
        return Ok(());
    };
    let Some(mut row) = db(SubscriptionTermRow::renewed_by(conn, phase.id).await)? else {
        return Ok(());
    };
    let months = Months::new(u32::try_from(row.renewal_months).unwrap_or(1));
    row.term_end = row
        .term_end
        .checked_add_months(months)
        .ok_or_else(|| invalid("the term's next end is out of range"))?;
    attach_renewal_tx(services, conn, &Actor::System, row).await?;
    Ok(())
}
