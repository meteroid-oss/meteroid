//! A subscription's grants as read (ADR 0019 §6): each grant of the offers applied to it, its
//! window's dates there, its status and the engine rows it wrote.
use std::collections::HashMap;

use common_domain::ids::{OfferApplicationId, OfferGrantId, SubscriptionId, TenantId};
use diesel_models::PgConn;
use diesel_models::offers::{OfferApplicationItemRow, OfferApplicationRow};
use itertools::Itertools;

use super::{catalog, db};
use crate::StoreResult;
use crate::domain::offers::{GrantStatus, GrantTarget, OfferApplication, SubscriptionGrant};

pub(crate) async fn subscription_grants_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscriptions: &[SubscriptionId],
) -> StoreResult<HashMap<SubscriptionId, Vec<SubscriptionGrant>>> {
    let applications =
        db(OfferApplicationRow::list_by_subscriptions(conn, &tenant_id, subscriptions).await)?;
    if applications.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<OfferApplicationId> = applications.iter().map(|a| a.id).collect();
    let items = db(OfferApplicationItemRow::list_by_applications(conn, &ids).await)?;
    let grant_ids: Vec<OfferGrantId> = applications
        .iter()
        .flat_map(|a| a.grant_ids.iter().flatten().copied())
        .unique()
        .collect();
    let grants = catalog::grants_by_ids_tx(conn, tenant_id, &grant_ids).await?;

    let mut add_ons = HashMap::new();
    let with_add_ons: Vec<OfferApplicationId> = items
        .iter()
        .filter(|i| i.kind == "ADD_ON")
        .map(|i| i.application_id)
        .collect();
    let with_add_on_subscriptions: Vec<SubscriptionId> = applications
        .iter()
        .filter(|a| with_add_ons.contains(&a.id))
        .map(|a| a.subscription_id)
        .unique()
        .collect();
    for subscription in with_add_on_subscriptions {
        for item in crate::services::subscriptions::timeline_items::list_items_tx(
            conn,
            tenant_id,
            subscription,
        )
        .await?
        {
            add_ons.insert(item.id, item.lineage);
        }
    }

    // Credit runs as funded: from the moment its subscription started, which a late checkout moves.
    let credit_ids: Vec<common_domain::ids::CreditGrantId> = items
        .iter()
        .filter_map(|i| match GrantTarget::parse(&i.kind, i.target_id) {
            Some(GrantTarget::CreditGrant(id)) => Some(id),
            _ => None,
        })
        .collect();
    let funded: HashMap<common_domain::ids::CreditGrantId, _> = db(
        diesel_models::customer_credit::CustomerCreditGrantRow::list_by_ids_scoped(
            conn,
            tenant_id,
            &credit_ids,
        )
        .await,
    )?
    .into_iter()
    .map(|row| (row.grant.id, (row.grant.effective_at, row.grant.expires_at)))
    .collect();

    let today = common_clock::now().date_naive();
    let mut out: HashMap<SubscriptionId, Vec<SubscriptionGrant>> = HashMap::new();
    for row in applications {
        let application = OfferApplication::try_from(row)?;
        for grant_id in &application.grant_ids {
            let Some(grant) = grants.get(grant_id) else {
                continue;
            };
            let written: Vec<&OfferApplicationItemRow> = items
                .iter()
                .filter(|i| i.application_id == application.id && i.grant_id == *grant_id)
                .collect();
            let targets = written
                .iter()
                .filter_map(|i| GrantTarget::parse(&i.kind, i.target_id))
                .map(|target| match target {
                    GrantTarget::AddOn { item_id, .. } => GrantTarget::AddOn {
                        item_id,
                        add_on: add_ons.get(&item_id).copied(),
                    },
                    target => target,
                })
                .collect();
            let (from, planned_until) = written
                .iter()
                .find_map(|i| match GrantTarget::parse(&i.kind, i.target_id) {
                    Some(GrantTarget::CreditGrant(id)) => funded.get(&id).map(|(from, until)| {
                        (from.date_naive(), until.map(|until| until.date_naive()))
                    }),
                    _ => None,
                })
                .unwrap_or_else(|| {
                    grant
                        .window
                        .dates(application.start_on, application.billing_start_on)
                });
            // Once written, its end is the one its items store (R-178), the sweep's.
            let until = if written.is_empty() {
                planned_until
            } else {
                written
                    .iter()
                    .filter_map(|i| i.ends_at)
                    .min()
                    .map(|at| at.date())
            };
            let status = if application.withdrawn_at.is_some() {
                GrantStatus::Withdrawn
            } else if written.iter().any(|i| i.ended_at.is_some())
                || until.is_some_and(|until| until <= today)
            {
                GrantStatus::Ended
            } else if application.started_at.is_none() || from > today {
                GrantStatus::Scheduled
            } else {
                GrantStatus::Active
            };
            out.entry(application.subscription_id)
                .or_default()
                .push(SubscriptionGrant {
                    grant: grant.clone(),
                    application: application.clone(),
                    status,
                    from,
                    until,
                    targets,
                });
        }
    }
    Ok(out)
}
