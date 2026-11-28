//! Offers (ADR 0019): the catalog and plans' start offers (`catalog`), the one resolution of a
//! creation's offer with its trial (`resolve`), what an offer writes on a subscription and when
//! (`apply`), and a subscription's grants as read (`grants`).
pub(crate) mod apply;
pub(crate) mod catalog;
pub(crate) mod follow;
pub(crate) mod grants;
pub(crate) mod resolve;

use std::collections::HashMap;

use common_domain::actor::Actor;
use common_domain::ids::{OfferId, PlanId, SubscriptionId, TenantId};
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::offers::{
    Offer, OfferNew, OfferPatch, OfferSource, OfferUse, PlanStartOffer, StartOfferChange,
    SubscriptionGrant,
};
use crate::services::{Services, ServicesEdge};

pub(crate) use apply::{end_with_subscription_tx, materialize_started_tx};
pub(crate) use resolve::resolve_tx;

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::into)
}

fn invalid(message: impl Into<String>) -> error_stack::Report<crate::errors::StoreError> {
    error_stack::Report::new(crate::errors::StoreError::InvalidArgument(message.into()))
}

impl ServicesEdge {
    /// A page of the catalog's offers, by id after `after`.
    pub async fn list_offers(
        &self,
        tenant_id: TenantId,
        after: Option<OfferId>,
        include_archived: bool,
        limit: i64,
    ) -> StoreResult<Vec<Offer>> {
        let mut conn = self.store.get_conn().await?;
        let rows = db(
            diesel_models::trial_offers::TrialOfferRow::list_standalone_after(
                &mut conn,
                &tenant_id,
                after,
                include_archived,
                limit,
            )
            .await,
        )?;
        catalog::offers_of_rows(&mut conn, tenant_id, rows).await
    }

    /// The tenant's offer `id`: a catalog offer, a plan's own or a coupon's.
    pub async fn find_offer(&self, tenant_id: TenantId, id: OfferId) -> StoreResult<Offer> {
        let mut conn = self.store.get_conn().await?;
        catalog::find_tx(&mut conn, tenant_id, id).await
    }

    pub async fn create_offer(&self, tenant_id: TenantId, offer: OfferNew) -> StoreResult<Offer> {
        self.store
            .transaction(|conn| {
                async move { catalog::create_tx(conn, tenant_id, offer, true).await }.scope_boxed()
            })
            .await
    }

    /// A catalog offer edited, for applications to come (R-170).
    pub async fn update_offer(
        &self,
        tenant_id: TenantId,
        id: OfferId,
        patch: OfferPatch,
    ) -> StoreResult<Offer> {
        self.store
            .transaction(|conn| {
                async move { catalog::update_tx(conn, tenant_id, id, patch).await }.scope_boxed()
            })
            .await
    }

    /// Archived, a catalog offer is given no more; the plans it was the start offer of.
    pub async fn archive_offer(
        &self,
        tenant_id: TenantId,
        id: OfferId,
    ) -> StoreResult<Vec<PlanId>> {
        self.store
            .transaction(|conn| {
                async move { catalog::archive_tx(conn, tenant_id, id).await }.scope_boxed()
            })
            .await
    }

    pub async fn offers_used_by(
        &self,
        tenant_id: TenantId,
        offers: &[OfferId],
    ) -> StoreResult<HashMap<OfferId, OfferUse>> {
        let mut conn = self.store.get_conn().await?;
        catalog::used_by_tx(&mut conn, tenant_id, offers).await
    }

    /// The start offer of each of `plans` that has one.
    pub async fn plan_start_offers(
        &self,
        tenant_id: TenantId,
        plans: &[PlanId],
    ) -> StoreResult<HashMap<PlanId, PlanStartOffer>> {
        let mut conn = self.store.get_conn().await?;
        catalog::plan_start_offers_tx(&mut conn, tenant_id, plans).await
    }

    pub async fn set_plan_start_offer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        plan_id: PlanId,
        change: StartOfferChange,
    ) -> StoreResult<Option<PlanStartOffer>> {
        self.services
            .set_plan_start_offer(actor, tenant_id, plan_id, change)
            .await
    }

    /// An offer without a trial applied to a running subscription now (R-174).
    pub async fn apply_offer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        offer_id: OfferId,
        source: OfferSource,
    ) -> StoreResult<Vec<SubscriptionGrant>> {
        self.services
            .apply_offer(actor, tenant_id, subscription_id, offer_id, source)
            .await
    }

    pub async fn subscription_grants(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Vec<SubscriptionGrant>> {
        Ok(self
            .subscriptions_grants(tenant_id, &[subscription_id])
            .await?
            .remove(&subscription_id)
            .unwrap_or_default())
    }

    pub async fn subscriptions_grants(
        &self,
        tenant_id: TenantId,
        subscriptions: &[SubscriptionId],
    ) -> StoreResult<HashMap<SubscriptionId, Vec<SubscriptionGrant>>> {
        let mut conn = self.store.get_conn().await?;
        grants::subscription_grants_tx(&mut conn, tenant_id, subscriptions).await
    }
}

impl Services {
    pub(in crate::services) async fn set_plan_start_offer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        plan_id: PlanId,
        change: StartOfferChange,
    ) -> StoreResult<Option<PlanStartOffer>> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    catalog::set_plan_start_offer_tx(
                        &self.store.internal,
                        conn,
                        actor,
                        tenant_id,
                        plan_id,
                        change,
                    )
                    .await?;
                    Ok(catalog::plan_start_offers_tx(conn, tenant_id, &[plan_id])
                        .await?
                        .remove(&plan_id))
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn apply_offer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        offer_id: OfferId,
        source: OfferSource,
    ) -> StoreResult<Vec<SubscriptionGrant>> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let application = apply::apply_now_tx(
                        self,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        offer_id,
                        source,
                    )
                    .await?;
                    Ok(
                        grants::subscription_grants_tx(conn, tenant_id, &[subscription_id])
                            .await?
                            .remove(&subscription_id)
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|grant| grant.application.id == application)
                            .collect(),
                    )
                }
                .scope_boxed()
            })
            .await
    }
}
