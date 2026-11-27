//! Trials (ADR 0015 §3): the plan's trial and the catalog of offers (`catalog`), the one resolution
//! of a creation's trial (`resolve`), what a customer is given once (`eligibility`), and a running
//! trial moved (`running`). Billing reads the subscription's trial fact, never an offer.
pub(crate) mod catalog;
pub(crate) mod eligibility;
pub(crate) mod resolve;
pub(crate) mod running;

use common_domain::actor::Actor;
use common_domain::ids::{AliasOr, PlanId, SubscriptionId, TenantId, TrialOfferId};

use crate::StoreResult;
use crate::domain::trials::{
    PlanTrial, SubscriptionTrial, TrialMove, TrialOffer, TrialOfferNew, TrialOfferPatch,
};
use crate::services::ServicesEdge;

pub(crate) use resolve::{PlanRef, resolve_trial_tx};
pub(crate) use running::trial_of_tx;

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::into)
}

fn invalid(message: impl Into<String>) -> error_stack::Report<crate::errors::StoreError> {
    error_stack::Report::new(crate::errors::StoreError::InvalidArgument(message.into()))
}

impl ServicesEdge {
    /// The trial of each of `plans` that has one.
    pub async fn plan_trials(
        &self,
        tenant_id: TenantId,
        plans: &[PlanId],
    ) -> StoreResult<std::collections::HashMap<PlanId, PlanTrial>> {
        let mut conn = self.store.get_conn().await?;
        catalog::plan_trials_tx(&mut conn, tenant_id, plans).await
    }

    /// The plan's trial set to a catalog offer, or removed.
    pub async fn set_plan_trial_offer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        plan_id: PlanId,
        offer: Option<TrialOfferId>,
    ) -> StoreResult<Option<PlanTrial>> {
        self.services
            .set_plan_trial_offer(actor, tenant_id, plan_id, offer)
            .await
    }

    pub async fn create_trial_offer(
        &self,
        tenant_id: TenantId,
        offer: TrialOfferNew,
    ) -> StoreResult<TrialOffer> {
        self.services.create_trial_offer(tenant_id, offer).await
    }

    /// An offer's terms edited, for trials to come (R-153).
    pub async fn update_trial_offer(
        &self,
        tenant_id: TenantId,
        id: TrialOfferId,
        patch: TrialOfferPatch,
    ) -> StoreResult<TrialOffer> {
        self.services.update_trial_offer(tenant_id, id, patch).await
    }

    /// Archived, an offer is no longer given nor any plan's trial; the plans it was the trial of.
    pub async fn archive_trial_offer(
        &self,
        tenant_id: TenantId,
        id: TrialOfferId,
    ) -> StoreResult<Vec<PlanId>> {
        self.services.archive_trial_offer(tenant_id, id).await
    }

    pub async fn list_trial_offers(&self, tenant_id: TenantId) -> StoreResult<Vec<TrialOffer>> {
        let mut conn = self.store.get_conn().await?;
        db(diesel_models::trial_offers::TrialOfferRow::list(&mut conn, &tenant_id).await)?
            .into_iter()
            .map(TrialOffer::try_from)
            .collect()
    }

    pub async fn list_trial_offers_after(
        &self,
        tenant_id: TenantId,
        after: Option<TrialOfferId>,
        include_archived: bool,
        limit: i64,
    ) -> StoreResult<Vec<TrialOffer>> {
        let mut conn = self.store.get_conn().await?;
        db(diesel_models::trial_offers::TrialOfferRow::list_after(
            &mut conn,
            &tenant_id,
            after,
            include_archived,
            limit,
        )
        .await)?
        .into_iter()
        .map(TrialOffer::try_from)
        .collect()
    }

    /// The tenant's offer of `id`, or coded `code`.
    pub async fn find_trial_offer(
        &self,
        tenant_id: TenantId,
        id_or_code: AliasOr<TrialOfferId>,
    ) -> StoreResult<TrialOffer> {
        let mut conn = self.store.get_conn().await?;
        catalog::find_tx(&mut conn, tenant_id, &id_or_code).await
    }

    /// The plans each of `offers` is the trial of.
    pub async fn plans_of_trial_offers(
        &self,
        tenant_id: TenantId,
        offers: &[TrialOfferId],
    ) -> StoreResult<std::collections::HashMap<TrialOfferId, Vec<PlanId>>> {
        let mut conn = self.store.get_conn().await?;
        catalog::plans_of_tx(&mut conn, tenant_id, offers).await
    }

    /// The subscription's trial, as its fact records it, and the offer it came from.
    pub async fn subscription_trial(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Option<(SubscriptionTrial, Option<TrialOffer>)>> {
        let mut conn = self.store.get_conn().await?;
        let Some(trial) = trial_of_tx(&mut conn, subscription_id).await? else {
            return Ok(None);
        };
        let offer = match trial.offer_id {
            Some(id) => db(diesel_models::trial_offers::TrialOfferRow::find(
                &mut conn, &tenant_id, id,
            )
            .await)?
            .map(TrialOffer::try_from)
            .transpose()?,
            None => None,
        };
        Ok(Some((trial, offer)))
    }

    /// A running trial extended, or ended today (R-153).
    pub async fn move_trial(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        to: TrialMove,
    ) -> StoreResult<SubscriptionTrial> {
        self.services
            .move_trial(actor, tenant_id, subscription_id, to)
            .await
    }
}
