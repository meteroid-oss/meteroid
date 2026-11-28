//! What an offer writes on a subscription, and when (R-174). At its creation: the application,
//! the applied coupons of its discount and credit grants, its dated add-ons; once it starts: the
//! credit funded and the entitlement rows. A subscription that never starts loses them all.
use chrono::{DateTime, Months, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    AppliedCouponId, BaseId, CouponId, OfferApplicationId, OfferGrantId, OfferId, SubscriptionId,
    TenantId,
};
use diesel_models::PgConn;
use diesel_models::applied_coupons::{AppliedCouponRow, AppliedCouponRowNew};
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::offers::{OfferApplicationItemRow, OfferApplicationRow, OfferGrantRow};
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use error_stack::Report;
use itertools::Itertools;

use super::{catalog, db, invalid};
use crate::StoreResult;
use crate::domain::coupons::CouponCreditScope;
use crate::domain::credit::{CreditCost, GrantClassification, GrantScope};
use crate::domain::entitlements::EntitlementSpec;
use crate::domain::offers::{
    AddOnEnd, GrantAnchor, GrantDefinition, GrantDuration, GrantKind, GrantTarget, OfferGrant,
    OfferSource, ResolvedOffer, kind_and_window_of_row,
};
use crate::domain::outbox_event::{GrantEndedEvent, OfferAppliedEvent, OutboxEvent};
use crate::domain::trials::ResolvedTrial;
use crate::domain::usage_alert_events::GrantEndReason;
use crate::errors::{OfferRefusal, StoreError};
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::customer_credit::{
    self as credit, CreditFunding, DEFAULT_PRIORITY, source,
};
use crate::services::Services;

const APPLICATION_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x2f6b_8c14_93d7_4e0a_b5c1_7a9e_04d3_6f28);

/// One per subscription, offer and coupon: a replay writes nothing (R-173).
fn application_id(
    subscription_id: SubscriptionId,
    offer_id: OfferId,
    coupon_id: Option<CouponId>,
) -> OfferApplicationId {
    let identity = format!(
        "{}|{}|{}",
        subscription_id.as_uuid(),
        offer_id.as_uuid(),
        coupon_id
            .map(|id| id.as_uuid().to_string())
            .unwrap_or_default()
    );
    uuid::Uuid::new_v5(&APPLICATION_NAMESPACE, identity.as_bytes()).into()
}

fn midnight(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(NaiveTime::MIN).and_utc()
}

/// A subscription created, as its offers read it.
pub(crate) struct Created<'a> {
    pub subscription: &'a SubscriptionRowNew,
    pub trial: Option<&'a ResolvedTrial>,
    pub offer: Option<&'a ResolvedOffer>,
    /// The applied coupons its codes wrote.
    pub coupons: &'a [AppliedCouponRowNew],
}

/// The subscription an application is written on.
#[derive(Clone, Copy)]
struct Owner {
    id: SubscriptionId,
    tenant_id: TenantId,
    customer_id: common_domain::ids::CustomerId,
    current_period_start: NaiveDate,
}

impl Owner {
    fn of(row: &SubscriptionRowNew) -> Self {
        Owner {
            id: row.id,
            tenant_id: row.tenant_id,
            customer_id: row.customer_id,
            current_period_start: row.current_period_start,
        }
    }
}

fn started(subscription: &SubscriptionRowNew) -> bool {
    match subscription.status {
        SubscriptionStatusEnum::Active => true,
        SubscriptionStatusEnum::TrialActive => !subscription.pending_checkout,
        _ => false,
    }
}

/// The offers of subscriptions just inserted recorded (R-174), in the caller's transaction and
/// after their coupons were redeemed; those that started get the rest at once, as of `at`.
pub(in crate::services) async fn record_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    created: &[Created<'_>],
    at: DateTime<Utc>,
) -> StoreResult<()> {
    for one in created {
        let subscription = one.subscription;
        let tenant_id = subscription.tenant_id;
        let start_on = subscription.start_date;
        let billing_start = subscription.billing_start_date.unwrap_or(start_on);
        let billing_start_on = one
            .trial
            .filter(|trial| trial.free)
            .map_or(billing_start, |trial| trial.until(billing_start));
        let mut events = Vec::new();

        if let Some(offer) = one.offer {
            let source = match offer.source {
                OfferSource::Plan => OfferSource::Plan,
                _ => OfferSource::of_actor(actor),
            };
            let application = insert_application_tx(
                conn,
                actor,
                Owner::of(subscription),
                offer.offer_id,
                None,
                source,
                &offer
                    .grants
                    .iter()
                    .map(|grant| grant.id)
                    .collect::<Vec<_>>(),
                start_on,
                billing_start_on,
                at.naive_utc(),
            )
            .await?
            .ok_or_else(|| {
                OfferRefusal::AlreadyApplied
                    .report("this offer is already applied to the subscription")
            })?;
            write_at_creation_tx(
                services,
                conn,
                actor,
                Owner::of(subscription),
                &application,
                &offer.grants,
            )
            .await?;
            events.push(applied_event(&application));
        }

        if !one.coupons.is_empty() {
            let coupon_ids: Vec<CouponId> =
                one.coupons.iter().map(|c| c.coupon_id).unique().collect();
            let offers = catalog::offers_of_coupons_tx(conn, tenant_id, &coupon_ids).await?;
            let grants: Vec<OfferGrantRow> =
                db(OfferGrantRow::list_by_coupons(conn, &coupon_ids).await)?;
            for applied in one.coupons {
                let Some(offer_id) = offers.get(&applied.coupon_id).copied() else {
                    continue;
                };
                let given: Vec<&OfferGrantRow> = grants
                    .iter()
                    .filter(|g| g.offer_id == offer_id && g.coupon_id == Some(applied.coupon_id))
                    .collect();
                let grant_ids: Vec<OfferGrantId> = given.iter().map(|g| g.id).collect();
                let application = insert_application_tx(
                    conn,
                    actor,
                    Owner::of(subscription),
                    offer_id,
                    Some(applied.coupon_id),
                    OfferSource::Coupon,
                    &grant_ids,
                    start_on,
                    billing_start_on,
                    at.naive_utc(),
                )
                .await?
                .ok_or_else(|| {
                    OfferRefusal::AlreadyApplied
                        .report("this offer is already applied to the subscription")
                })?;
                let items = given
                    .iter()
                    .map(|grant| {
                        let (kind, window) = kind_and_window_of_row(grant)?;
                        Ok(item(
                            &application,
                            grant.id,
                            GrantTarget::AppliedCoupon(applied.id),
                            window.ends_at(
                                kind,
                                application.start_on,
                                application.billing_start_on,
                                None,
                            ),
                        ))
                    })
                    .collect::<StoreResult<Vec<OfferApplicationItemRow>>>()?;
                db(OfferApplicationItemRow::insert_batch(conn, &items).await)?;
                events.push(applied_event(&application));
            }
        }

        crate::repositories::outbox::publish_outbox_events_tx(conn, events).await?;
        if started(subscription) {
            materialize_started_tx(
                conn,
                &*services.store.hooks.credit,
                tenant_id,
                subscription.id,
                at,
            )
            .await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_application_tx(
    conn: &mut PgConn,
    actor: &Actor,
    subscription: Owner,
    offer_id: OfferId,
    coupon_id: Option<CouponId>,
    source: OfferSource,
    grant_ids: &[OfferGrantId],
    start_on: NaiveDate,
    billing_start_on: NaiveDate,
    applied_at: NaiveDateTime,
) -> StoreResult<Option<OfferApplicationRow>> {
    db(OfferApplicationRow {
        id: application_id(subscription.id, offer_id, coupon_id),
        tenant_id: subscription.tenant_id,
        offer_id,
        customer_id: subscription.customer_id,
        subscription_id: subscription.id,
        coupon_id,
        source: source.as_str().to_string(),
        start_on,
        billing_start_on,
        applied_at,
        actor_type: actor.actor_type().into(),
        actor_uuid: actor.as_uuid(),
        actor_alias: actor.actor_alias(),
        started_at: None,
        withdrawn_at: None,
        grant_ids: grant_ids.iter().copied().map(Some).collect(),
    }
    .insert(conn)
    .await)
}

fn item(
    application: &OfferApplicationRow,
    grant_id: OfferGrantId,
    target: GrantTarget,
    ends_at: Option<NaiveDateTime>,
) -> OfferApplicationItemRow {
    let (kind, target_id) = target.columns();
    OfferApplicationItemRow {
        application_id: application.id,
        grant_id,
        tenant_id: application.tenant_id,
        kind: kind.to_string(),
        target_id,
        created_at: common_clock::now().naive_utc(),
        ended_at: None,
        ends_at,
    }
}

fn applied_event(application: &OfferApplicationRow) -> OutboxEvent {
    OutboxEvent::OfferApplied(Box::new(OfferAppliedEvent {
        id: OfferAppliedEvent::event_id(application.id),
        tenant_id: application.tenant_id,
        customer_id: application.customer_id,
        subscription_id: application.subscription_id,
        offer_id: application.offer_id,
        application_id: application.id,
        coupon_id: application.coupon_id,
        source: application.source.clone(),
        grant_ids: application.grant_ids.iter().flatten().copied().collect(),
        applied_at: application.applied_at,
    }))
}

/// What an application writes when it is made: an applied coupon per discount or credit grant
/// (its months counted from billing's start, R-089), a dated add-on per add-on grant.
async fn write_at_creation_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    subscription: Owner,
    application: &OfferApplicationRow,
    grants: &[OfferGrant],
) -> StoreResult<()> {
    let effective_from = crate::services::subscriptions::coupon_months_from(
        application.billing_start_on,
        subscription.current_period_start,
    );
    let mut items = Vec::new();
    let mut applied = Vec::new();
    for grant in grants {
        let ends_at = grant.window.ends_at(
            grant.kind(),
            application.start_on,
            application.billing_start_on,
            None,
        );
        match &grant.definition {
            GrantDefinition::Discount(_) | GrantDefinition::Credit(_) => {
                let coupon_id = grant
                    .coupon_id
                    .ok_or_else(|| invalid(format!("grant {} has no coupon", grant.id)))?;
                let row = AppliedCouponRowNew {
                    id: AppliedCouponId::new(),
                    coupon_id,
                    customer_id: subscription.customer_id,
                    subscription_id: subscription.id,
                    is_active: true,
                    applied_amount: None,
                    applied_count: None,
                    last_applied_at: None,
                    effective_from,
                    discounts_from: None,
                };
                items.push(item(
                    application,
                    grant.id,
                    GrantTarget::AppliedCoupon(row.id),
                    ends_at,
                ));
                applied.push(row);
            }
            GrantDefinition::AddOn(add_on) => {
                if let Some(target) = write_add_on_tx(
                    services,
                    conn,
                    actor,
                    subscription,
                    application,
                    grant,
                    add_on,
                )
                .await?
                {
                    items.push(item(application, grant.id, target, ends_at));
                }
            }
            GrantDefinition::Entitlement(_) => {}
        }
    }
    if !applied.is_empty() {
        db(AppliedCouponRow::insert_batch(conn, applied.iter().collect()).await)?;
        db(AppliedCouponRow::snapshot_terms(
            conn,
            &applied.iter().map(|row| row.id).collect::<Vec<_>>(),
        )
        .await)?;
    }
    db(OfferApplicationItemRow::insert_batch(conn, &items).await)
}

/// A dated add-on, free over its window: a window started before today is given from today, one
/// already over gives nothing.
async fn write_add_on_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    subscription: Owner,
    application: &OfferApplicationRow,
    grant: &OfferGrant,
    add_on: &crate::domain::offers::GrantAddOn,
) -> StoreResult<Option<GrantTarget>> {
    let (from, until) = grant
        .window
        .dates(application.start_on, application.billing_start_on);
    let today = common_clock::now().date_naive();
    let Some(until) = until.filter(|until| *until > today && *until > from) else {
        return Ok(None);
    };
    let item = crate::services::subscriptions::timeline_items::write_item_tx(
        services,
        conn,
        actor,
        subscription.tenant_id,
        subscription.id,
        &crate::domain::subscription_phases::TimelineItemInput {
            add_on_id: add_on.add_on_id,
            existing: None,
            quantity: i32::try_from(add_on.quantity).unwrap_or(i32::MAX),
            start_date: from.max(today),
            until,
            terms: crate::domain::subscription_phases::TimelineItemTerms {
                free: true,
                end: match add_on.at_end {
                    AddOnEnd::Revert => crate::domain::subscription_phases::ItemEnd::Revert,
                    AddOnEnd::Convert => {
                        crate::domain::subscription_phases::ItemEnd::Convert { price: None }
                    }
                },
                prorated: true,
            },
            label: None,
            reason: Some("Offer".to_string()),
        },
    )
    .await?;
    Ok(Some(GrantTarget::AddOn {
        item_id: item.id,
        add_on: Some(item.lineage),
    }))
}

/// What the offers applied to a subscription give once it starts, at `at`: their credit funded
/// and their entitlement rows. Every transition starting it calls it; a replay writes nothing.
pub(crate) async fn materialize_started_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    at: DateTime<Utc>,
) -> StoreResult<()> {
    let applications: Vec<OfferApplicationRow> =
        db(OfferApplicationRow::list_by_subscriptions(conn, &tenant_id, &[subscription_id]).await)?
            .into_iter()
            .filter(|a| a.started_at.is_none() && a.withdrawn_at.is_none())
            .collect();
    if applications.is_empty() {
        return Ok(());
    }
    let grant_ids: Vec<OfferGrantId> = applications
        .iter()
        .flat_map(|a| a.grant_ids.iter().flatten().copied())
        .unique()
        .collect();
    let grants = catalog::grants_by_ids_tx(conn, tenant_id, &grant_ids).await?;
    let ids: Vec<OfferApplicationId> = applications.iter().map(|a| a.id).collect();
    let recorded = db(OfferApplicationItemRow::list_by_applications(conn, &ids).await)?;
    let subscription =
        db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
            .subscription;

    let mut items = Vec::new();
    for application in &applications {
        for grant_id in application.grant_ids.iter().flatten() {
            let Some(grant) = grants.get(grant_id) else {
                continue;
            };
            let (from, until) = grant
                .window
                .dates(application.start_on, application.billing_start_on);
            match &grant.definition {
                GrantDefinition::Entitlement(entitlement) => {
                    if until.is_some_and(|until| until <= from) {
                        continue;
                    }
                    let spec = EntitlementSpec {
                        effect: entitlement.effect,
                        valid_from: Some(midnight(from)),
                        valid_until: until.map(midnight),
                        reason: Some("Offer".to_string()),
                        ..EntitlementSpec::new(entitlement.feature_id, entitlement.value.clone())
                    };
                    if let Some(row) =
                        crate::repositories::entitlements::insert_entitlement_spec_if_free_tx(
                            conn,
                            spec,
                            common_domain::ids::EntitlementEntityId::Subscription(subscription_id),
                            tenant_id,
                        )
                        .await?
                    {
                        items.push(item(
                            application,
                            grant.id,
                            GrantTarget::Entitlement(row.id),
                            grant.window.ends_at(
                                grant.kind(),
                                application.start_on,
                                application.billing_start_on,
                                None,
                            ),
                        ));
                    }
                }
                GrantDefinition::Credit(definition) => {
                    let Some(applied_coupon) = recorded.iter().find_map(|i| {
                        (i.application_id == application.id
                            && i.grant_id == grant.id
                            && i.kind == "APPLIED_COUPON")
                            .then_some(i.target_id)
                    }) else {
                        continue;
                    };
                    // A code's credit is funded as coupons' is, with its checks: it is linked.
                    if application.coupon_id.is_some() {
                        if let Some(funded) =
                            diesel_models::customer_credit::CustomerCreditGrantRow::find_by_source(
                                conn,
                                tenant_id,
                                source::COUPON,
                                applied_coupon,
                            )
                            .await?
                        {
                            items.push(item(
                                application,
                                grant.id,
                                GrantTarget::CreditGrant(funded.id),
                                grant.window.ends_at(
                                    grant.kind(),
                                    application.start_on,
                                    application.billing_start_on,
                                    funded.expires_at,
                                ),
                            ));
                        }
                        continue;
                    }
                    if subscription.currency != definition.currency {
                        return Err(invalid(format!(
                            "the offer grants {} credit, but the subscription is billed in {}",
                            definition.currency, subscription.currency
                        )));
                    }
                    let effective_at = match grant.window.anchor {
                        GrantAnchor::Start => at,
                        GrantAnchor::BillingStart => midnight(application.billing_start_on).max(at),
                    };
                    let expires_at = match grant.window.duration {
                        GrantDuration::Days(days) => chrono::Duration::try_days(i64::from(days))
                            .and_then(|term| effective_at.checked_add_signed(term)),
                        GrantDuration::Months(months) => {
                            effective_at.checked_add_months(Months::new(months))
                        }
                        _ => None,
                    };
                    let amount_minor = crate::domain::coupons::CouponDiscount::Credit {
                        currency: definition.currency.clone(),
                        amount: definition.amount,
                        expires_in_days: None,
                        scope: definition.scope,
                    }
                    .credit_minor()
                    .ok_or_else(|| {
                        invalid(
                            "promotional credit has more decimal places than its currency allows",
                        )
                    })?;
                    let funded = credit::fund_tx(
                        conn,
                        controls,
                        CreditFunding {
                            tenant_id,
                            customer_id: application.customer_id,
                            currency: definition.currency.clone(),
                            scope: match definition.scope {
                                CouponCreditScope::Customer => GrantScope::customer_wide(),
                                CouponCreditScope::Subscription => GrantScope {
                                    subscription_id: Some(subscription_id),
                                    ..GrantScope::customer_wide()
                                },
                            },
                            amount_minor,
                            classification: GrantClassification::Promotional,
                            priority: DEFAULT_PRIORITY,
                            effective_at,
                            expires_at,
                            occurred_at: at,
                            now: at,
                            source_kind: source::COUPON,
                            source_id: applied_coupon,
                            cost: Some(CreditCost::given(&definition.currency)),
                            rollover: None,
                            invoice_id: None,
                            line_local_id: None,
                            credit_note_id: None,
                            reason: "Offer".to_string(),
                            reference: None,
                            created_by: None,
                            pays_usage_from: None,
                        },
                    )
                    .await?;
                    items.push(item(
                        application,
                        grant.id,
                        GrantTarget::CreditGrant(funded.id),
                        grant.window.ends_at(
                            grant.kind(),
                            application.start_on,
                            application.billing_start_on,
                            funded.expires_at,
                        ),
                    ));
                }
                GrantDefinition::Discount(_) | GrantDefinition::AddOn(_) => {}
            }
        }
    }
    db(OfferApplicationItemRow::insert_batch(conn, &items).await)?;
    db(OfferApplicationRow::mark_started(conn, &ids, at.naive_utc()).await)
}

/// A subscription that never started loses what its offers gave (R-174): its applied coupons
/// deactivated, its dated add-ons withdrawn, the applications withdrawn, `grant.ended` sent.
pub(in crate::services) async fn withdraw_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<()> {
    let now = common_clock::now().naive_utc();
    let withdrawn =
        db(
            OfferApplicationRow::withdraw_for_subscription(conn, &tenant_id, subscription_id, now)
                .await,
        )?;
    let ids: Vec<OfferApplicationId> = withdrawn.iter().map(|a| a.id).collect();
    let ended = db(OfferApplicationItemRow::end_for_applications(conn, &ids, now).await)?;
    for row in &ended {
        match GrantTarget::parse(&row.kind, row.target_id) {
            Some(GrantTarget::AppliedCoupon(id)) => {
                db(AppliedCouponRow::deactivate(conn, id).await)?;
            }
            Some(GrantTarget::AddOn { item_id, .. }) => {
                crate::services::subscriptions::timeline_items::withdraw_item_by_id_tx(
                    conn,
                    tenant_id,
                    subscription_id,
                    item_id,
                    "Withdrawn with its subscription",
                )
                .await?;
            }
            _ => {}
        }
    }
    publish_ended_tx(
        conn,
        &withdrawn,
        &ended,
        GrantEndReason::Withdrawn,
        tenant_id,
    )
    .await
}

/// The subscription ended: its grants end with it, but customer-wide credit, which outlives it
/// (R-175). The engines end their rows themselves; this records it and sends `grant.ended`.
pub(crate) async fn end_with_subscription_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    ended_at: NaiveDateTime,
) -> StoreResult<()> {
    let applications: Vec<OfferApplicationRow> =
        db(OfferApplicationRow::list_by_subscriptions(conn, &tenant_id, &[subscription_id]).await)?
            .into_iter()
            .filter(|a| a.withdrawn_at.is_none())
            .collect();
    if applications.is_empty() {
        return Ok(());
    }
    let ids: Vec<OfferApplicationId> = applications.iter().map(|a| a.id).collect();
    let items = db(OfferApplicationItemRow::list_by_applications(conn, &ids).await)?;
    let grant_ids: Vec<OfferGrantId> = items.iter().map(|i| i.grant_id).unique().collect();
    let grants = catalog::grants_by_ids_tx(conn, tenant_id, &grant_ids).await?;
    let outlives = |grant_id: &OfferGrantId| {
        grants.get(grant_id).is_some_and(|grant| {
            matches!(&grant.definition, GrantDefinition::Credit(c) if c.scope == CouponCreditScope::Customer)
        })
    };
    let open: Vec<&OfferApplicationItemRow> = items
        .iter()
        .filter(|i| i.ended_at.is_none() && !outlives(&i.grant_id))
        .collect();
    if open.is_empty() {
        return Ok(());
    }
    let ended = db(OfferApplicationItemRow::end_targets(
        conn,
        &open.iter().map(|i| i.target_id).collect::<Vec<_>>(),
        ended_at,
    )
    .await)?;
    publish_ended_tx(
        conn,
        &applications,
        &ended,
        GrantEndReason::SubscriptionEnded,
        tenant_id,
    )
    .await
}

pub(crate) async fn publish_ended_tx(
    conn: &mut PgConn,
    applications: &[OfferApplicationRow],
    ended: &[OfferApplicationItemRow],
    reason: GrantEndReason,
    tenant_id: TenantId,
) -> StoreResult<()> {
    let ids: Vec<OfferGrantId> = ended.iter().map(|i| i.grant_id).unique().collect();
    let grants = catalog::grants_by_ids_tx(conn, tenant_id, &ids).await?;
    let events: Vec<OutboxEvent> = ended
        .iter()
        .unique_by(|i| (i.application_id, i.grant_id))
        .filter_map(|i| {
            let application = applications.iter().find(|a| a.id == i.application_id)?;
            let kind = grants
                .get(&i.grant_id)
                .map_or(GrantKind::Discount, OfferGrant::kind);
            Some(OutboxEvent::GrantEnded(Box::new(GrantEndedEvent {
                id: GrantEndedEvent::event_id(application.id, i.grant_id),
                tenant_id: application.tenant_id,
                customer_id: application.customer_id,
                subscription_id: application.subscription_id,
                offer_id: application.offer_id,
                application_id: application.id,
                grant_id: i.grant_id,
                kind: kind.as_str().to_string(),
                reason,
                ended_at: i
                    .ended_at
                    .unwrap_or_else(|| common_clock::now().naive_utc()),
            })))
        })
        .collect();
    crate::repositories::outbox::publish_outbox_events_tx(conn, events).await
}

/// An offer applied to a started subscription now (R-174): its grants anchored on today, the
/// discounts on the next period; an offer with a trial is refused.
#[allow(clippy::too_many_arguments)]
pub(in crate::services) async fn apply_now_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    offer_id: OfferId,
    source: OfferSource,
) -> StoreResult<OfferApplicationId> {
    let offer = super::resolve::catalog_offer_tx(conn, tenant_id, offer_id).await?;
    if offer.trial.is_some() {
        return Err(OfferRefusal::TrialNotApplicable
            .report("an offer with a trial is given when a subscription is created"));
    }
    if offer.grants.is_empty() {
        return Err(invalid("the offer gives no grant"));
    }
    let lock = SubscriptionRow::lock_customer_of_tenant(conn, tenant_id, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
    let row = db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?;
    let sub = row.subscription;
    if !matches!(
        sub.status,
        SubscriptionStatusEnum::Active | SubscriptionStatusEnum::TrialActive
    ) {
        return Err(invalid("an offer is applied to a running subscription"));
    }
    if db(OfferApplicationRow::find(conn, &tenant_id, subscription_id, offer.id).await)?.is_some() {
        return Err(OfferRefusal::AlreadyApplied
            .report("this offer is already applied to the subscription"));
    }
    let plan =
        crate::services::trials::PlanRef::of_version_tx(conn, tenant_id, sub.plan_version_id)
            .await?;
    if offer.eligibility != crate::domain::trials::TrialEligibility::None
        && super::resolve::given_tx(conn, tenant_id, sub.customer_id, &offer, &plan).await?
    {
        return Err(OfferRefusal::AlreadyGiven.report(format!(
            "the customer has already had this offer ({})",
            offer.name
        )));
    }
    let today = common_clock::now().date_naive();
    let billing_start_on =
        crate::services::subscriptions::coupon_months_from_id_tx(conn, tenant_id, subscription_id)
            .await?
            .max(today);
    let owner = Owner {
        id: sub.id,
        tenant_id,
        customer_id: sub.customer_id,
        current_period_start: sub.current_period_start,
    };
    let application = insert_application_tx(
        conn,
        actor,
        owner,
        offer.id,
        None,
        source,
        &offer.grants.iter().map(|g| g.id).collect::<Vec<_>>(),
        today,
        billing_start_on,
        common_clock::now().naive_utc(),
    )
    .await?
    .ok_or_else(|| {
        OfferRefusal::AlreadyApplied.report("this offer is already applied to the subscription")
    })?;
    write_at_creation_tx(services, conn, actor, owner, &application, &offer.grants).await?;
    crate::repositories::outbox::publish_outbox_events_tx(conn, vec![applied_event(&application)])
        .await?;
    materialize_started_tx(
        conn,
        &*services.store.hooks.credit,
        tenant_id,
        subscription_id,
        common_clock::now(),
    )
    .await?;
    Ok(application.id)
}
