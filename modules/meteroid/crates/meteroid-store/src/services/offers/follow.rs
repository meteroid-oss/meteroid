//! A free trial moved (extended, or ended now): the grants counted from billing's start, or lasting
//! until it, follow its end (R-177). Those counted from the subscription's start stay as written.
use chrono::{DateTime, Months, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    OfferApplicationId, OfferGrantId, ScheduledEventId, SubscriptionId, TenantId,
};
use diesel_models::PgConn;
use diesel_models::applied_coupons::AppliedCouponRow;
use diesel_models::offers::{OfferApplicationItemRow, OfferApplicationRow};
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_phases::SubscriptionTimelineItemRow;
use itertools::Itertools;

use super::{catalog, db, invalid};
use crate::StoreResult;
use crate::domain::offers::{GrantAnchor, GrantDefinition, GrantDuration, GrantKind, GrantTarget};
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use crate::services::Services;

fn midnight(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(chrono::NaiveTime::MIN).and_utc()
}

/// The applications counting billing's start from the trial's end `until`, not withdrawn.
async fn bound_to_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    until: NaiveDate,
) -> StoreResult<Vec<OfferApplicationRow>> {
    Ok(
        db(OfferApplicationRow::list_by_subscriptions(conn, &tenant_id, &[subscription_id]).await)?
            .into_iter()
            .filter(|a| a.withdrawn_at.is_none() && a.billing_start_on == until)
            .collect(),
    )
}

/// The scheduled changes of the dated add-ons the offers bound to the trial ending `until` gave:
/// moved with the trial, they never stand in its way.
pub(crate) async fn trial_bound_changes_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    until: NaiveDate,
) -> StoreResult<Vec<ScheduledEventId>> {
    let applications = bound_to_tx(conn, tenant_id, subscription_id, until).await?;
    if applications.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<OfferApplicationId> = applications.iter().map(|a| a.id).collect();
    let item_ids: Vec<uuid::Uuid> =
        db(OfferApplicationItemRow::list_by_applications(conn, &ids).await)?
            .into_iter()
            .filter(|i| i.ended_at.is_none() && i.kind == "ADD_ON")
            .map(|i| i.target_id)
            .collect();
    if item_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(
        db(SubscriptionTimelineItemRow::list_live(conn, &tenant_id, subscription_id).await)?
            .into_iter()
            .filter(|item| item_ids.contains(&item.id))
            .flat_map(|item| [item.start_change_id, Some(item.end_change_id)])
            .flatten()
            .collect(),
    )
}

/// The engine rows of the offers bound to the trial ending `from` dated again for its end `to`,
/// in the caller's transaction, after the trial's fact moved.
#[allow(clippy::too_many_arguments)]
pub(in crate::services) async fn follow_trial_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    current_period_start: NaiveDate,
    from: NaiveDate,
    to: NaiveDate,
) -> StoreResult<()> {
    if from == to {
        return Ok(());
    }
    let applications = bound_to_tx(conn, tenant_id, subscription_id, from).await?;
    if applications.is_empty() {
        return Ok(());
    }
    let ids: Vec<OfferApplicationId> = applications.iter().map(|a| a.id).collect();
    db(OfferApplicationRow::move_billing_start(conn, &ids, to).await)?;
    let items: Vec<OfferApplicationItemRow> =
        db(OfferApplicationItemRow::list_by_applications(conn, &ids).await)?
            .into_iter()
            .filter(|i| i.ended_at.is_none())
            .collect();
    let grant_ids: Vec<OfferGrantId> = items.iter().map(|i| i.grant_id).unique().collect();
    let grants = catalog::grants_by_ids_tx(conn, tenant_id, &grant_ids).await?;
    let now = common_clock::now();
    let mut wallets = Vec::new();
    for item in &items {
        let (Some(grant), Some(application)) = (
            grants.get(&item.grant_id),
            applications.iter().find(|a| a.id == item.application_id),
        ) else {
            continue;
        };
        let before = grant.window.dates(application.start_on, from);
        let after = grant.window.dates(application.start_on, to);
        if before == after {
            continue;
        }
        // Credit's end is its funded grant's, known once re-dated (`None`: left as it was).
        let mut redated_expiry = None;
        match (
            GrantTarget::parse(&item.kind, item.target_id),
            &grant.definition,
        ) {
            (Some(GrantTarget::AppliedCoupon(id)), GrantDefinition::Discount(_)) => {
                let on =
                    crate::services::subscriptions::coupon_months_from(to, current_period_start);
                db(AppliedCouponRow::move_effective_from(conn, id, on).await)?;
            }
            (Some(GrantTarget::CreditGrant(id)), GrantDefinition::Credit(_))
                if grant.window.anchor == GrantAnchor::BillingStart =>
            {
                let effective_at = midnight(to).max(now);
                let expires_at = match grant.window.duration {
                    GrantDuration::Days(days) => chrono::Duration::try_days(i64::from(days))
                        .and_then(|term| effective_at.checked_add_signed(term)),
                    GrantDuration::Months(months) => {
                        effective_at.checked_add_months(Months::new(months))
                    }
                    _ => None,
                };
                if let Some(row) = crate::repositories::credit::customer_credit::redate_pending_tx(
                    conn,
                    tenant_id,
                    application.customer_id,
                    id,
                    effective_at,
                    expires_at,
                    now,
                )
                .await?
                {
                    redated_expiry = Some(row.expires_at);
                    wallets.push(row.account_id);
                }
            }
            (Some(GrantTarget::Entitlement(lineage)), GrantDefinition::Entitlement(_)) => {
                crate::repositories::entitlements::redate_entitlement_tx(
                    conn,
                    actor,
                    tenant_id,
                    lineage,
                    Some(midnight(after.0)),
                    after.1.map(midnight),
                )
                .await?;
            }
            (Some(GrantTarget::AddOn { item_id, .. }), GrantDefinition::AddOn(_)) => {
                if let Some(until) = after.1 {
                    move_add_on_end_tx(
                        services,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        item_id,
                        until,
                    )
                    .await?;
                }
            }
            _ => {}
        }
        let ends_at = match (grant.kind(), redated_expiry) {
            (GrantKind::Credit, None) => continue,
            (kind, expiry) => {
                grant
                    .window
                    .ends_at(kind, application.start_on, to, expiry.flatten())
            }
        };
        if ends_at != item.ends_at {
            db(
                OfferApplicationItemRow::set_ends_at(conn, &item.kind, item.target_id, ends_at)
                    .await,
            )?;
        }
    }
    if wallets.is_empty() {
        return Ok(());
    }
    crate::repositories::credit::customer_credit::balance_changed_tx(
        conn,
        &*services.store.hooks.credit,
        tenant_id,
        &wallets,
        now,
    )
    .await
}

/// A dated add-on's end change written again on `until`, its rows as they were.
async fn move_add_on_end_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    item_id: uuid::Uuid,
    until: NaiveDate,
) -> StoreResult<()> {
    let Some(item) =
        db(SubscriptionTimelineItemRow::list_live(conn, &tenant_id, subscription_id).await)?
            .into_iter()
            .find(|item| item.id == item_id)
    else {
        return Ok(());
    };
    if item.until_date == until {
        return Ok(());
    }
    let change = db(SubscriptionChangeRow::get_by_id(conn, item.end_change_id, &tenant_id).await)?;
    let ScheduledEventData::ApplyPhase {
        component_close,
        component_insert,
        addon_close,
        addon_insert,
        ..
    } = ScheduledEvent::try_from(change.clone())?.event_data
    else {
        return Err(invalid(format!(
            "timeline item {item_id} ends with a change that is no boundary"
        )));
    };
    crate::services::facts::pending::withdraw_pending_tx(conn, &change, "Moved with the trial")
        .await?;
    // Ended now, before it started: nothing of it is given.
    if until <= item.start_date {
        if let Some(start) = item.start_change_id {
            let start = db(SubscriptionChangeRow::get_by_id(conn, start, &tenant_id).await)?;
            crate::services::facts::pending::withdraw_pending_tx(
                conn,
                &start,
                "Ended with the trial before it started",
            )
            .await?;
        }
        db(SubscriptionTimelineItemRow::withdraw(conn, item.id).await)?;
        return Ok(());
    }
    let end = crate::services::subscriptions::timeline_items::schedule_tx(
        services,
        conn,
        actor,
        tenant_id,
        subscription_id,
        item.id,
        until,
        crate::services::subscriptions::timeline_items::ChangeRows {
            component_close,
            component_insert,
            addon_close,
            addon_insert,
        },
    )
    .await?;
    db(SubscriptionTimelineItemRow::move_end(conn, item.id, until, end).await)
}
